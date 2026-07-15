// Native audio playback for the Bang voice overlay.
//
// The overlay's spoken answers are now synthesized *in-app* by the local Piper
// neural TTS (see crates/voice_tts) — this file just plays the resulting PCM.
// We hand a self-contained WAV byte buffer to AVAudioPlayer, which gives us a
// clean stop (for barge-in / interrupt) and a completion delegate.
//
// Controlled from Rust (app/src/overlay/platform_mac.rs) via bang_tts_play_wav /
// bang_tts_stop. When playback finishes on its own, we call the Rust entrypoint
// bang_tts_did_finish so the overlay resumes listening (we don't listen while
// speaking, to avoid the mic transcribing our own voice). A stop / interrupt /
// replacement does NOT fire that callback.
//
// (Previously this shelled out to `/usr/bin/say`. We moved off `say` because it
// runs in a separate process — which meant its audio couldn't be echo-cancelled
// and its voice wasn't consistent across machines. Owning the PCM in-app also
// sets up software echo-cancellation for hands-free barge-in in Phase 2.)
//
// Compiled with ARC (see app/build.rs). Entry points run on the main thread.

#import <AVFoundation/AVFoundation.h>
#import <Foundation/Foundation.h>

// Implemented in Rust (app/src/overlay/platform_mac.rs).
extern void bang_tts_did_finish(void);

@interface BangTtsDelegate : NSObject <AVAudioPlayerDelegate>
@end

// The currently-playing player, or nil. Identity distinguishes a natural finish
// (still the current player) from a stop/replacement (player swapped out), so we
// only resume listening when playback finished on its own.
static AVAudioPlayer *gPlayer = nil;
static BangTtsDelegate *gDelegate = nil;

@implementation BangTtsDelegate
- (void)audioPlayerDidFinishPlaying:(AVAudioPlayer *)player successfully:(BOOL)flag {
    (void)flag;
    dispatch_async(dispatch_get_main_queue(), ^{
        if (gPlayer == player) {
            gPlayer = nil;
            bang_tts_did_finish();
        }
    });
}
@end

void bang_tts_stop(void) {
    AVAudioPlayer *player = gPlayer;
    if (player != nil) {
        // Clear first so the delegate sees it's no longer current and does not
        // fire the "finished" callback.
        gPlayer = nil;
        [player stop];
    }
}

void bang_tts_play_wav(const uint8_t *bytes, size_t len) {
    if (bytes == NULL || len == 0) {
        return;
    }
    // Interrupt anything already speaking so answers don't overlap.
    bang_tts_stop();

    // dataWithBytes copies, so the caller's buffer can be freed after this call.
    NSData *data = [NSData dataWithBytes:bytes length:len];
    NSError *error = nil;
    AVAudioPlayer *player = [[AVAudioPlayer alloc] initWithData:data error:&error];
    if (player == nil) {
        NSLog(@"[bang] tts: failed to init player: %@", error);
        return;
    }
    if (gDelegate == nil) {
        gDelegate = [[BangTtsDelegate alloc] init];
    }
    player.delegate = gDelegate;
    [player prepareToPlay];
    if (![player play]) {
        NSLog(@"[bang] tts: play failed");
        return;
    }
    gPlayer = player;
}
