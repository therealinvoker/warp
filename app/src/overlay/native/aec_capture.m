// Echo-cancelled microphone capture for hands-free voice barge-in (macOS).
//
// Uses AVAudioEngine with the input node's voice-processing (AUVoiceProcessingIO)
// enabled, which applies acoustic echo cancellation / noise suppression / AGC.
// The point is to let the overlay keep the mic live *while the agent's answer is
// being read aloud* without the mic transcribing the agent's own voice. Captured
// audio is converted to 24kHz mono PCM16 LE (the format the Realtime pipeline
// streams) and handed to Rust one frame at a time via `bang_aec_frame`.
//
// NOTE (path A): voice processing cancels the audio rendered through its own
// output element. Our TTS is `/usr/bin/say` in a separate process, so whether
// the OS cancels it here is unverified — see the plan. If it doesn't, the caller
// detects the leakage and we escalate. `bang_aec_start` returns a non-zero status
// when voice processing / the engine can't be set up, and the Rust side
// additionally watchdogs for frames actually arriving (falling back to plain
// cpal capture if the tap never fires).
//
// Compiled with ARC (see app/build.rs). `bang_aec_start` / `bang_aec_stop` are
// called on the main thread; the tap block runs on a realtime audio thread, so
// `bang_aec_frame` must be cheap + non-blocking on the Rust side.

#import <AVFoundation/AVFoundation.h>
#import <stddef.h>
#import <stdint.h>

// Implemented in Rust (app/src/overlay/platform_mac.rs). Pushes a 24kHz mono
// PCM16 LE frame into the Realtime stream channel (non-blocking).
extern void bang_aec_frame(const uint8_t *bytes, size_t len);

// Status codes returned by bang_aec_start (kept in sync with platform_mac.rs).
enum {
    BANG_AEC_OK = 0,
    BANG_AEC_UNSUPPORTED_OS = 1,
    BANG_AEC_VP_UNAVAILABLE = 2,
    BANG_AEC_BAD_FORMAT = 3,
    BANG_AEC_CONVERTER_FAILED = 4,
    BANG_AEC_ENGINE_START_FAILED = 5,
};

static AVAudioEngine *gEngine = nil;
static AVAudioConverter *gConverter = nil;
static AVAudioFormat *gOutFormat = nil;
// Last failure detail, surfaced to Rust (which logs it to warp-oss.log) since
// this process's NSLog isn't captured by unified logging.
static char gLastError[512] = {0};

static void bangSetError(NSString *msg) {
    if (msg == nil) {
        gLastError[0] = '\0';
        return;
    }
    const char *utf8 = msg.UTF8String;
    if (utf8 == NULL) {
        gLastError[0] = '\0';
        return;
    }
    snprintf(gLastError, sizeof(gLastError), "%s", utf8);
}

const char *bang_aec_last_error(void) {
    return gLastError;
}
// Held as the base AVAudioNode type (available since 10.10) so this file-scope
// global doesn't require the 10.15 AVAudioSourceNode type; it's only created
// inside the availability-guarded start path.
static AVAudioNode *gSilenceNode = nil;
static BOOL gLoggedFirstFrame = NO;

// Build + start an engine with voice processing on the input node.
//
// A voice-processing engine is duplex: the I/O only runs (and only *pulls the
// mic*) when its render/output side is active. With no output, the input tap
// fires with all-zero buffers (silence) on some devices. But wiring the mic to
// the output makes the *mic itself* the echo-cancellation reference, so the AEC
// cancels the mic against itself — also silence. The fix is to render a
// dedicated **silence source** (zeros) into the output: the I/O runs (mic gets
// pulled) and the AEC reference is true silence, so the captured mic audio
// passes through. Returns a BANG_AEC_* status.
API_AVAILABLE(macos(10.15))
static int bangStartEngine(void) {
    AVAudioEngine *engine = [[AVAudioEngine alloc] init];
    AVAudioInputNode *input = engine.inputNode;

    bangSetError(nil);

    NSError *vpError = nil;
    if (![input setVoiceProcessingEnabled:YES error:&vpError]) {
        NSLog(@"[bang] AEC: voice processing unavailable: %@", vpError);
        bangSetError(vpError.localizedDescription);
        return BANG_AEC_VP_UNAVAILABLE;
    }

    // Use the node's negotiated *output* format for the tap/connection; with
    // voice processing enabled this is the reliable format to key off.
    AVAudioFormat *nodeFormat = [input outputFormatForBus:0];
    if (nodeFormat == nil || nodeFormat.sampleRate <= 0 || nodeFormat.channelCount == 0) {
        NSLog(@"[bang] AEC: input has no valid format: %@", nodeFormat);
        return BANG_AEC_BAD_FORMAT;
    }

    AVAudioFormat *outFormat = [[AVAudioFormat alloc] initWithCommonFormat:AVAudioPCMFormatInt16
                                                                sampleRate:24000.0
                                                                  channels:1
                                                               interleaved:YES];
    AVAudioConverter *converter = [[AVAudioConverter alloc] initFromFormat:nodeFormat
                                                                  toFormat:outFormat];
    if (converter == nil) {
        NSLog(@"[bang] AEC: converter init failed for %@", nodeFormat);
        return BANG_AEC_CONVERTER_FAILED;
    }

    // Render silence into the output so the duplex voice-processing I/O runs and
    // pulls the mic, with a true-silence AEC reference (not the mic itself). The
    // VP unit is a single duplex unit, so its input and output elements must run
    // at the same sample rate — render at the input node's own format and wire
    // straight to the output node (bypassing the main mixer, whose hardware rate
    // otherwise mismatches the VP rate and fails unit init, -10875).
    AVAudioFormat *renderFormat = nodeFormat;
    AVAudioSourceNode *silence = [[AVAudioSourceNode alloc]
        initWithFormat:renderFormat
           renderBlock:^OSStatus(BOOL *isSilence, const AudioTimeStamp *timestamp,
                                 AVAudioFrameCount frameCount, AudioBufferList *outputData) {
             (void)timestamp;
             (void)frameCount;
             *isSilence = YES;
             for (UInt32 i = 0; i < outputData->mNumberBuffers; i++) {
                 if (outputData->mBuffers[i].mData != NULL) {
                     memset(outputData->mBuffers[i].mData, 0, outputData->mBuffers[i].mDataByteSize);
                 }
             }
             return noErr;
           }];
    @try {
        [engine attachNode:silence];
        [engine connect:silence to:engine.outputNode format:renderFormat];
    } @catch (NSException *ex) {
        NSLog(@"[bang] AEC: silence source wiring failed: %@", ex);
        bangSetError([NSString stringWithFormat:@"silence wiring: %@ (renderFormat %@)", ex.reason,
                                                renderFormat]);
    }
    gSilenceNode = silence;

    gEngine = engine;
    gConverter = converter;
    gOutFormat = outFormat;
    gLoggedFirstFrame = NO;

    [input installTapOnBus:0
                bufferSize:4096
                    format:nodeFormat
                     block:^(AVAudioPCMBuffer *inBuffer, AVAudioTime *when) {
                       (void)when;
                       if (gConverter == nil || gOutFormat == nil || inBuffer.frameLength == 0) {
                           return;
                       }
                       double ratio = gOutFormat.sampleRate / inBuffer.format.sampleRate;
                       AVAudioFrameCount capacity =
                           (AVAudioFrameCount)(inBuffer.frameLength * ratio) + 1024;
                       AVAudioPCMBuffer *outBuffer =
                           [[AVAudioPCMBuffer alloc] initWithPCMFormat:gOutFormat
                                                        frameCapacity:capacity];
                       if (outBuffer == nil) {
                           return;
                       }
                       __block BOOL provided = NO;
                       AVAudioConverterInputBlock inputBlock =
                           ^AVAudioBuffer *(AVAudioPacketCount inNumberOfPackets,
                                            AVAudioConverterInputStatus *outStatus) {
                         (void)inNumberOfPackets;
                         if (provided) {
                             *outStatus = AVAudioConverterInputStatus_NoDataNow;
                             return nil;
                         }
                         provided = YES;
                         *outStatus = AVAudioConverterInputStatus_HaveData;
                         return inBuffer;
                       };
                       NSError *convError = nil;
                       AVAudioConverterOutputStatus status =
                           [gConverter convertToBuffer:outBuffer
                                                 error:&convError
                                    withInputFromBlock:inputBlock];
                       if (status == AVAudioConverterOutputStatus_Error) {
                           return;
                       }
                       AVAudioFrameCount frames = outBuffer.frameLength;
                       const int16_t *samples =
                           outBuffer.int16ChannelData ? outBuffer.int16ChannelData[0] : NULL;
                       if (frames > 0 && samples != NULL) {
                           if (!gLoggedFirstFrame) {
                               gLoggedFirstFrame = YES;
                               NSLog(@"[bang] AEC: first frame delivered (%u samples)",
                                     (unsigned)frames);
                           }
                           bang_aec_frame((const uint8_t *)samples,
                                          (size_t)frames * sizeof(int16_t));
                       }
                     }];

    [engine prepare];
    NSError *startError = nil;
    if (![engine startAndReturnError:&startError]) {
        NSLog(@"[bang] AEC: engine start failed: %@", startError);
        bangSetError(startError.localizedDescription);
        [input removeTapOnBus:0];
        gEngine = nil;
        gConverter = nil;
        gOutFormat = nil;
        gSilenceNode = nil;
        return BANG_AEC_ENGINE_START_FAILED;
    }
    NSLog(@"[bang] AEC: engine started (input format %@)", nodeFormat);
    return BANG_AEC_OK;
}

int bang_aec_start(void) {
    if (gEngine != nil) {
        return BANG_AEC_OK;
    }
    if (@available(macOS 10.15, *)) {
        return bangStartEngine();
    }
    return BANG_AEC_UNSUPPORTED_OS;
}

void bang_aec_stop(void) {
    if (gEngine != nil) {
        [gEngine.inputNode removeTapOnBus:0];
        [gEngine stop];
        if (gSilenceNode != nil) {
            @try {
                [gEngine detachNode:gSilenceNode];
            } @catch (NSException *e) {
                (void)e;
            }
        }
        gEngine = nil;
        gConverter = nil;
        gOutFormat = nil;
        gSilenceNode = nil;
        gLoggedFirstFrame = NO;
    }
}
