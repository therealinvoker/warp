// Native macOS "puck" windows for the Bang voice + annotation overlay.
//
// Borderless, always-on-top, transparent panels:
//   - the MIC puck (kind 0): gradient + microphone glyph; reflects listening /
//     paused state and modulates its glow with the live mic level. Clicking it
//     pauses/resumes recording. Draggable.
//   - the SETTINGS gear: a small floating button pinned below-and-right of the
//     mic puck (follows it on drag). Clicking it toggles a small settings
//     popover with switches for "Auto-submit" and "Read answers aloud".
//   - the RESULT box: a semi-transparent text panel above the mic puck. Its
//     bottom-left holds a small SUBMIT button (kind 1, an up-arrow) that sends
//     the transcript; the top-right holds the expand/collapse control.
//
// Controlled from Rust (app/src/overlay/platform_mac.rs) via the
// `bang_overlay_puck_*` C functions. Clicks are routed back into the app via the
// Rust entrypoint `bang_overlay_puck_clicked(kind)`. Compiled with ARC (see
// app/build.rs). All functions run on the main thread (AppKit requirement).

#import <Cocoa/Cocoa.h>
#import <CoreGraphics/CoreGraphics.h>
#import <stdbool.h>
#import <unistd.h>

// Implemented in Rust (app/src/overlay/platform_mac.rs).
extern void bang_overlay_puck_clicked(int kind);
extern void bang_overlay_box_edited(const char *utf8);
extern void bang_overlay_auto_submit_clicked(void);
extern void bang_overlay_voice_toggled(void);
// Called when a color swatch in the settings popover is chosen; Rust persists
// the index and pushes it back via bang_overlay_set_puck_color.
extern void bang_overlay_puck_color_clicked(int index);
// Called when a language is chosen in the settings popover; Rust persists the
// ISO code (empty = auto-detect) and reconnects the transcription session.
extern void bang_overlay_language_selected(const char *code);
// Called when the verbosity slider (0-10) changes in the settings popover; Rust
// persists it (shared with Settings > AI) and sends it with each agent request.
extern void bang_overlay_verbosity_selected(int level);

// Voice overlay transcription languages offered in the settings popover.
// `code` is the ISO-639-1 code sent to the transcriber ("" = auto-detect).
typedef struct {
    const char *code;
    const char *name;
} BangLangOption;
static const BangLangOption kLangOptions[] = {
    {"en", "English"},   {"", "Auto-detect"}, {"es", "Spanish"},
    {"fr", "French"},    {"de", "German"},    {"it", "Italian"},
    {"pt", "Portuguese"},{"nl", "Dutch"},     {"ja", "Japanese"},
    {"ko", "Korean"},    {"zh", "Chinese"},   {"hi", "Hindi"},
    {"ru", "Russian"},   {"ar", "Arabic"},
};
static const int kLangOptionCount = (int)(sizeof(kLangOptions) / sizeof(kLangOptions[0]));
// The currently-selected language code (pushed from Rust via
// bang_overlay_set_language); applied to the popup when the panel is built.
static char gLangCode[8] = "en";
// Current response-verbosity level (0-10), pushed from Rust via
// bang_overlay_set_verbosity; applied to the slider when the panel is built.
static int gVerbosity = 5;

// Start/stop the result box's "thinking" shimmer (defined in the box section).
static void bangBoxSetThinking(BOOL on);
// Refresh the settings popover's verbosity readout (defined in that section).
static void bangUpdateVerbosityLabel(void);
// Annotation canvas callbacks (see the "Annotation canvas" section below).
extern void bang_overlay_canvas_done(double x, double y, double w, double h);
extern void bang_overlay_canvas_cancel(void);

enum { BangPuckKindMic = 0, BangPuckKindSubmit = 1, BangPuckKindPencil = 2 };

// All three row pucks (pencil, mic, settings) share a single size so they sit on
// a grid-aligned row with a consistent gap between them.
static const CGFloat kPuckSize = 44.0;
static const CGFloat kPuckGap = 10.0;

// The Spotlight-style text input pill sits to the right of the settings gear on
// the same row. It's a rounded pill at its single-line minimum (height ==
// kPuckSize) and grows upward as content is added.
static const CGFloat kBoxPillWidth = 340.0;
static const CGFloat kBoxTextPadLeft = 16.0;   // left inset before the text
static const CGFloat kBoxSubmitSize = 30.0;    // submit (up-arrow) puck
static const CGFloat kBoxSubmitPad = 7.0;      // gap from the pill's right edge
static const CGFloat kBoxTextInsetV = 14.0;    // top/bottom text inset (centers 1 line)
static const CGFloat kBoxHistInsetV = 10.0;    // top/bottom inset for the history region
static const CGFloat kBoxDividerH = 1.0;       // separator between history and input

// Preset accent palette shared by all pucks. The chosen index is persisted on
// the Rust side (AISettings.voice_overlay_puck_color) and pushed back here via
// bang_overlay_set_puck_color. Keep in sync with any docs referencing presets.
//
// A preset is either a solid color (`gradient == NO`, only the r/g/b start color
// is used) or a two-stop diagonal gradient (`gradient == YES`, start r/g/b ->
// end r2/g2/b2). Solids are listed first, gradients after; the settings popover
// renders them in two labeled rows ("Color" / "Gradient").
typedef struct {
    const char *name;
    CGFloat r, g, b;    // solid / gradient start color
    CGFloat r2, g2, b2; // gradient end color (ignored when gradient == NO)
    BOOL gradient;
} BangPuckPreset;

static const BangPuckPreset kPuckPresets[] = {
    // Solids.
    {"Pink", 1.0, 0.176, 0.471, 0, 0, 0, NO},   // #FF2D78
    {"Blue", 0.302, 0.486, 1.0, 0, 0, 0, NO},   // #4D7CFF
    {"Green", 0.204, 0.780, 0.349, 0, 0, 0, NO},
    {"Purple", 0.686, 0.322, 0.871, 0, 0, 0, NO},
    {"Graphite", 0.28, 0.28, 0.32, 0, 0, 0, NO},
    // Gradients (start -> end, drawn top-left to bottom-right).
    {"Sunset", 1.0, 0.235, 0.443, 1.0, 0.6, 0.2, YES},    // pink -> orange
    {"Ocean", 0.235, 0.51, 1.0, 0.157, 0.86, 0.78, YES},  // blue -> teal
    {"Violet", 0.545, 0.31, 0.98, 1.0, 0.29, 0.6, YES},   // purple -> pink
    {"Lime", 0.4, 0.85, 0.3, 0.11, 0.63, 0.51, YES},      // green -> teal
    {"Ember", 0.98, 0.42, 0.16, 0.86, 0.13, 0.36, YES},   // orange -> red
};
static const NSInteger kPuckPresetCount =
    (NSInteger)(sizeof(kPuckPresets) / sizeof(kPuckPresets[0]));

// Index into kPuckPresets for the current accent color (defaults to Pink).
static NSInteger gPuckColorIndex = 0;

// Fills an oval `path` with the accent preset at `idx`: a solid color, or a
// diagonal two-stop gradient (top-left -> bottom-right) for gradient presets.
// A `dim` preset (mic paused) always fills flat graphite.
static void bangFillOvalPreset(NSBezierPath *path, NSInteger idx, BOOL dim) {
    if (dim) {
        [[NSColor colorWithSRGBRed:0.24 green:0.24 blue:0.28 alpha:1.0] setFill];
        [path fill];
        return;
    }
    if (idx < 0 || idx >= kPuckPresetCount) {
        idx = 0;
    }
    BangPuckPreset p = kPuckPresets[idx];
    if (p.gradient) {
        NSColor *start = [NSColor colorWithSRGBRed:p.r green:p.g blue:p.b alpha:1.0];
        NSColor *end = [NSColor colorWithSRGBRed:p.r2 green:p.g2 blue:p.b2 alpha:1.0];
        NSGradient *grad = [[NSGradient alloc] initWithStartingColor:start endingColor:end];
        [NSGraphicsContext saveGraphicsState];
        [path addClip];
        [grad drawInRect:path.bounds angle:-45.0];
        [NSGraphicsContext restoreGraphicsState];
    } else {
        [[NSColor colorWithSRGBRed:p.r green:p.g blue:p.b alpha:1.0] setFill];
        [path fill];
    }
}

// Draws the shared puck chrome — a filled accent circle (solid or gradient) — so
// every puck (pencil / mic / settings) looks identical.
static void bangDrawPuckChrome(NSRect bounds, BOOL dim) {
    NSRect fillRect = NSInsetRect(bounds, 3.0, 3.0);
    NSBezierPath *oval = [NSBezierPath bezierPathWithOvalInRect:fillRect];
    bangFillOvalPreset(oval, gPuckColorIndex, dim);
}

// True when the view is rendering under the dark system appearance.
static BOOL bangIsDarkMode(NSView *view) {
    NSAppearance *appearance =
        view != nil ? view.effectiveAppearance : NSApp.effectiveAppearance;
    NSAppearanceName match = [appearance
        bestMatchFromAppearancesWithNames:@[ NSAppearanceNameAqua, NSAppearanceNameDarkAqua ]];
    return [match isEqualToString:NSAppearanceNameDarkAqua];
}

// Icon glyph tint: light grey in dark mode, white in light mode.
static NSColor *bangPuckIconColor(NSView *view) {
    return bangIsDarkMode(view) ? [NSColor colorWithWhite:0.72 alpha:1.0]
                                : [NSColor whiteColor];
}

// Repositions the floating settings gear and pencil relative to the mic puck.
// Defined below; forward-declared so the mic puck's drag handler can call them.
static void bangPositionGear(void);
static void bangPositionPencil(void);

// Group-drag: dragging any overlay piece (mic / gear / pencil) moves the whole
// cluster (and the result box, if visible) together. Defined below; forward-
// declared so every puck view's drag handler can call them.
static void bangBeginGroupDrag(void);
static void bangUpdateGroupDrag(void);

/// Circular puck content view. Draws itself based on `kind` and state, and
/// implements manual dragging (so a click can be distinguished from a drag).
@interface BangPuckView : NSView
@property(nonatomic, assign) int kind;
@property(nonatomic, assign) BOOL listening;
@property(nonatomic, assign) BOOL paused;
@property(nonatomic, assign) double level; // 0..~1 RMS
@property(nonatomic, assign) BOOL thinking; // submit puck: agent is working
@property(nonatomic, assign) double phase;  // spotlight animation phase (radians)
@property(nonatomic, strong) NSTimer *spinTimer;
@property(nonatomic, assign) BOOL didDrag;
// When embedded as a subview (e.g. the submit button inside the result box) the
// puck must not drag its host window; a click always fires.
@property(nonatomic, assign) BOOL embedded;
@end

@implementation BangPuckView

- (void)drawMicInRect:(NSRect)bounds {
    CGFloat s = MIN(bounds.size.width, bounds.size.height);
    NSColor *color =
        self.paused ? [NSColor colorWithWhite:0.55 alpha:1.0] : bangPuckIconColor(self);

    // Prefer the system SF Symbol so the mic matches the weight/quality of the
    // pencil and gear glyphs (which are system font glyphs).
    if (@available(macOS 11.0, *)) {
        NSImage *symbol = [NSImage imageWithSystemSymbolName:@"mic.fill"
                                    accessibilityDescription:nil];
        if (symbol != nil) {
            NSImageSymbolConfiguration *cfg =
                [NSImageSymbolConfiguration configurationWithPointSize:s * 0.5
                                                               weight:NSFontWeightRegular];
            symbol = [symbol imageWithSymbolConfiguration:cfg] ?: symbol;
            NSSize isz = symbol.size;
            // Tint the template glyph to `color` and draw it centered.
            NSImage *tinted = [NSImage imageWithSize:isz
                                             flipped:NO
                                      drawingHandler:^BOOL(NSRect rect) {
                                        [symbol drawInRect:rect];
                                        [color set];
                                        NSRectFillUsingOperation(rect, NSCompositingOperationSourceAtop);
                                        return YES;
                                      }];
            [tinted drawInRect:NSMakeRect(NSMidX(bounds) - isz.width / 2.0,
                                          NSMidY(bounds) - isz.height / 2.0, isz.width, isz.height)];
            return;
        }
    }

    // Fallback (pre-macOS 11): a simple hand-drawn microphone.
    CGFloat cx = NSMidX(bounds);
    CGFloat cy = NSMidY(bounds);
    CGFloat lw = s * 0.07;
    CGFloat capW = s * 0.26;
    NSRect capRect = NSMakeRect(cx - capW / 2.0, cy - s * 0.12, capW, s * 0.42);
    [color setFill];
    [[NSBezierPath bezierPathWithRoundedRect:capRect xRadius:capW / 2.0 yRadius:capW / 2.0] fill];
    NSBezierPath *cradle = [NSBezierPath bezierPath];
    cradle.lineWidth = lw;
    cradle.lineCapStyle = NSLineCapStyleRound;
    [cradle appendBezierPathWithArcWithCenter:NSMakePoint(cx, cy - s * 0.04)
                                       radius:s * 0.24
                                   startAngle:200.0
                                     endAngle:340.0];
    [color setStroke];
    [cradle stroke];
    NSBezierPath *stand = [NSBezierPath bezierPath];
    stand.lineWidth = lw;
    stand.lineCapStyle = NSLineCapStyleRound;
    CGFloat stemTopY = cy - s * 0.04 - s * 0.24;
    CGFloat baseY = cy - s * 0.40;
    [stand moveToPoint:NSMakePoint(cx, stemTopY)];
    [stand lineToPoint:NSMakePoint(cx, baseY)];
    [stand moveToPoint:NSMakePoint(cx - s * 0.12, baseY)];
    [stand lineToPoint:NSMakePoint(cx + s * 0.12, baseY)];
    [stand stroke];
}

- (void)drawUpArrowInRect:(NSRect)bounds {
    CGFloat s = MIN(bounds.size.width, bounds.size.height);
    CGFloat cx = NSMidX(bounds);
    CGFloat top = NSMidY(bounds) + s * 0.24;
    CGFloat bottom = NSMidY(bounds) - s * 0.24;
    CGFloat head = s * 0.16;
    NSBezierPath *arrow = [NSBezierPath bezierPath];
    arrow.lineWidth = s * 0.09;
    arrow.lineCapStyle = NSLineCapStyleRound;
    arrow.lineJoinStyle = NSLineJoinStyleRound;
    // Shaft.
    [arrow moveToPoint:NSMakePoint(cx, bottom)];
    [arrow lineToPoint:NSMakePoint(cx, top)];
    // Arrowhead.
    [arrow moveToPoint:NSMakePoint(cx - head, top - head)];
    [arrow lineToPoint:NSMakePoint(cx, top)];
    [arrow lineToPoint:NSMakePoint(cx + head, top - head)];
    [bangPuckIconColor(self) setStroke];
    [arrow stroke];
}

// A bright arc that sweeps around the puck while the agent is thinking.
- (void)drawSpotlightInRect:(NSRect)bounds {
    NSPoint center = NSMakePoint(NSMidX(bounds), NSMidY(bounds));
    CGFloat radius = MIN(bounds.size.width, bounds.size.height) / 2.0 - 2.0;
    CGFloat start = self.phase * 180.0 / M_PI;
    // Draw a fading tail of arc segments behind the leading edge.
    for (int i = 0; i < 12; i++) {
        CGFloat a0 = start - i * 10.0;
        CGFloat a1 = a0 + 10.0;
        CGFloat alpha = 0.9 - (i * 0.07);
        if (alpha < 0.0) {
            alpha = 0.0;
        }
        NSBezierPath *arc = [NSBezierPath bezierPath];
        arc.lineWidth = 3.0;
        arc.lineCapStyle = NSLineCapStyleRound;
        [arc appendBezierPathWithArcWithCenter:center radius:radius startAngle:a0 endAngle:a1];
        [[NSColor colorWithSRGBRed:1.0 green:1.0 blue:1.0 alpha:alpha] setStroke];
        [arc stroke];
    }
}

- (void)spinTick:(NSTimer *)timer {
    (void)timer;
    self.phase += 0.22;
    [self setNeedsDisplay:YES];
}

- (void)startSpin {
    if (self.spinTimer != nil) {
        return;
    }
    self.spinTimer = [NSTimer scheduledTimerWithTimeInterval:0.03
                                                      target:self
                                                    selector:@selector(spinTick:)
                                                    userInfo:nil
                                                     repeats:YES];
}

- (void)stopSpin {
    [self.spinTimer invalidate];
    self.spinTimer = nil;
    self.phase = 0.0;
    [self setNeedsDisplay:YES];
}

- (void)drawRect:(NSRect)dirtyRect {
    (void)dirtyRect;
    NSRect bounds = self.bounds;

    if (self.kind == BangPuckKindSubmit) {
        // Share the accent chrome so the send button matches the row pucks.
        bangDrawPuckChrome(bounds, NO);
        if (self.thinking) {
            [self drawSpotlightInRect:bounds];
        }
        [self drawUpArrowInRect:bounds];
        return;
    }

    // Mic puck. Draw only the shared chrome + glyph so it matches the pencil and
    // gear pucks — no outer ring/glow behind it. While the agent is working, the
    // same sweeping arc spins around the mic.
    bangDrawPuckChrome(bounds, self.paused);
    if (self.thinking) {
        [self drawSpotlightInRect:bounds];
    }
    [self drawMicInRect:bounds];
}

// Deliver the very first click even when Bang isn't the active app. The puck
// lives in a non-activating floating panel, so without this AppKit swallows the
// first click just to make the panel key and only the second click reaches
// mouseDown/mouseUp — which showed up as needing to double-click the mic puck to
// pause/resume. Returning YES routes the initial click straight to the puck.
- (BOOL)acceptsFirstMouse:(NSEvent *)event {
    (void)event;
    return YES;
}

// Manual dragging + click detection. `movableByWindowBackground` would swallow
// clicks, so we move the window ourselves and treat a mouse-up without drag as a
// click.
- (void)mouseDown:(NSEvent *)event {
    (void)event;
    self.didDrag = NO;
    // Embedded buttons (e.g. the box's submit) never drag; only the free-floating
    // mic puck starts a group drag.
    if (!self.embedded) {
        bangBeginGroupDrag();
    }
}

- (void)mouseDragged:(NSEvent *)event {
    (void)event;
    // Embedded buttons never drag their host window (e.g. the box's submit).
    if (self.embedded) {
        return;
    }
    self.didDrag = YES;
    // Move the whole overlay cluster (mic + gear + pencil + box) together.
    bangUpdateGroupDrag();
}

- (void)mouseUp:(NSEvent *)event {
    (void)event;
    if (self.embedded || !self.didDrag) {
        bang_overlay_puck_clicked(self.kind);
    }
}

@end

static NSPanel *gMicPuck = nil;
static NSPanel *gGearPuck = nil;
static NSPanel *gPencilPuck = nil;
// The submit button lives inside the result box (bottom-left), created in
// bangEnsureBox. The "thinking" spotlight is drawn on this view.
static BangPuckView *gBoxSubmitView = nil;
static void bangEnsureSettingsPanel(void);
static void bangToggleSettingsPanel(void);
static void bangSettingsPanelHide(void);

static NSPanel *bangMakePuck(int kind, NSRect frame) {
    NSPanel *panel = [[NSPanel alloc]
        initWithContentRect:frame
                  styleMask:(NSWindowStyleMaskBorderless | NSWindowStyleMaskNonactivatingPanel)
                    backing:NSBackingStoreBuffered
                      defer:NO];
    panel.level = NSFloatingWindowLevel;
    panel.opaque = NO;
    panel.backgroundColor = [NSColor clearColor];
    panel.hasShadow = YES;
    panel.collectionBehavior =
        NSWindowCollectionBehaviorCanJoinAllSpaces | NSWindowCollectionBehaviorFullScreenAuxiliary;
    panel.releasedWhenClosed = NO;

    BangPuckView *view = [[BangPuckView alloc] initWithFrame:NSMakeRect(0, 0, frame.size.width,
                                                                        frame.size.height)];
    view.kind = kind;
    panel.contentView = view;
    return panel;
}

// A borderless button that draws a gear glyph on a translucent circle. Clicking
// it toggles the settings popover.
@interface BangGearView : NSView {
    BOOL _didDrag;
}
@end
@implementation BangGearView
- (void)drawRect:(NSRect)dirtyRect {
    (void)dirtyRect;
    NSRect bounds = self.bounds;
    bangDrawPuckChrome(bounds, NO);
    NSString *glyph = @"\u2699\uFE0E"; // gear, text (non-emoji) presentation
    NSDictionary *attrs = @{
        NSForegroundColorAttributeName : bangPuckIconColor(self),
        NSFontAttributeName : [NSFont systemFontOfSize:MIN(bounds.size.width, bounds.size.height) * 0.62]
    };
    NSSize sz = [glyph sizeWithAttributes:attrs];
    [glyph drawAtPoint:NSMakePoint(NSMidX(bounds) - sz.width / 2.0, NSMidY(bounds) - sz.height / 2.0)
        withAttributes:attrs];
}
// Deliver the first click even when Bang isn't the active app (see
// BangPuckView's acceptsFirstMouse), so the gear opens settings on one click.
- (BOOL)acceptsFirstMouse:(NSEvent *)event {
    (void)event;
    return YES;
}
// Accept the mouse-down so the matching mouse-up is delivered here, and anchor a
// potential group drag.
- (void)mouseDown:(NSEvent *)event {
    (void)event;
    _didDrag = NO;
    bangBeginGroupDrag();
}
- (void)mouseDragged:(NSEvent *)event {
    (void)event;
    _didDrag = YES;
    bangUpdateGroupDrag();
}
- (void)mouseUp:(NSEvent *)event {
    (void)event;
    // A drag moves the cluster; only a clean click toggles settings.
    if (!_didDrag) {
        bangToggleSettingsPanel();
    }
}
@end

// Position the settings gear immediately to the right of the mic puck, on the
// same row (grid-aligned), clamped on-screen.
static void bangPositionGear(void) {
    if (gGearPuck == nil || gMicPuck == nil) {
        return;
    }
    NSRect mic = gMicPuck.frame;
    CGFloat x = NSMaxX(mic) + kPuckGap;
    CGFloat y = NSMinY(mic);
    NSScreen *screen = gMicPuck.screen ?: [NSScreen mainScreen];
    if (screen != nil) {
        NSRect visible = [screen visibleFrame];
        if (x + kPuckSize > NSMaxX(visible)) {
            x = NSMaxX(visible) - kPuckSize - 2.0;
        }
    }
    [gGearPuck setFrameOrigin:NSMakePoint(x, y)];
}

// A borderless button that draws a pencil glyph on a translucent circle.
// Clicking it starts the annotation canvas (routed through Rust as kind 2).
@interface BangPencilView : NSView {
    BOOL _didDrag;
}
@end
@implementation BangPencilView
- (void)drawRect:(NSRect)dirtyRect {
    (void)dirtyRect;
    NSRect bounds = self.bounds;
    bangDrawPuckChrome(bounds, NO);
    NSString *glyph = @"\u270E\uFE0E"; // lower-right pencil, text presentation
    NSDictionary *attrs = @{
        NSForegroundColorAttributeName : bangPuckIconColor(self),
        NSFontAttributeName : [NSFont systemFontOfSize:MIN(bounds.size.width, bounds.size.height) * 0.5]
    };
    NSSize sz = [glyph sizeWithAttributes:attrs];
    [glyph drawAtPoint:NSMakePoint(NSMidX(bounds) - sz.width / 2.0, NSMidY(bounds) - sz.height / 2.0)
        withAttributes:attrs];
}
// Deliver the first click even when Bang isn't the active app (see
// BangPuckView's acceptsFirstMouse), so the pencil launches on one click.
- (BOOL)acceptsFirstMouse:(NSEvent *)event {
    (void)event;
    return YES;
}
- (void)mouseDown:(NSEvent *)event {
    (void)event;
    _didDrag = NO;
    bangBeginGroupDrag();
}
- (void)mouseDragged:(NSEvent *)event {
    (void)event;
    _didDrag = YES;
    bangUpdateGroupDrag();
}
- (void)mouseUp:(NSEvent *)event {
    (void)event;
    // A drag moves the cluster; only a clean click starts the annotation canvas.
    if (!_didDrag) {
        bang_overlay_puck_clicked(BangPuckKindPencil);
    }
}
@end

// Position the pencil puck immediately to the left of the mic puck, on the same
// row (grid-aligned), clamped on-screen.
static void bangPositionPencil(void) {
    if (gPencilPuck == nil || gMicPuck == nil) {
        return;
    }
    NSRect mic = gMicPuck.frame;
    CGFloat x = NSMinX(mic) - kPuckGap - kPuckSize;
    CGFloat y = NSMinY(mic);
    NSScreen *screen = gMicPuck.screen ?: [NSScreen mainScreen];
    if (screen != nil) {
        NSRect visible = [screen visibleFrame];
        if (x < NSMinX(visible)) {
            x = NSMinX(visible) + 2.0;
        }
    }
    [gPencilPuck setFrameOrigin:NSMakePoint(x, y)];
}

static void bangEnsurePucks(void) {
    if (gMicPuck != nil) {
        return;
    }
    NSScreen *screen = [NSScreen mainScreen];
    NSRect visible = (screen != nil) ? [screen visibleFrame] : NSMakeRect(0, 0, 1440, 900);
    // All three row pucks share one size for a grid-aligned row.
    const CGFloat size = kPuckSize;
    const CGFloat margin = 24.0;

    // Anchor the row bottom-right, reserving room to the mic's right for the
    // settings gear AND the text pill (pencil sits to the mic's left):
    //   [pencil] [mic] [gear] [------ pill ------]
    // so the pill's right edge lands at the screen margin.
    CGFloat micX =
        NSMaxX(visible) - margin - kBoxPillWidth - 2.0 * kPuckGap - 2.0 * kPuckSize;
    CGFloat rowY = NSMinY(visible) + margin;
    NSRect micFrame = NSMakeRect(micX, rowY, size, size);
    gMicPuck = bangMakePuck(BangPuckKindMic, micFrame);

    // Floating settings gear, pinned to the right of the mic puck.
    gGearPuck = [[NSPanel alloc]
        initWithContentRect:NSMakeRect(0, 0, kPuckSize, kPuckSize)
                  styleMask:(NSWindowStyleMaskBorderless | NSWindowStyleMaskNonactivatingPanel)
                    backing:NSBackingStoreBuffered
                      defer:NO];
    gGearPuck.level = NSFloatingWindowLevel;
    gGearPuck.opaque = NO;
    gGearPuck.backgroundColor = [NSColor clearColor];
    gGearPuck.hasShadow = YES;
    gGearPuck.collectionBehavior =
        NSWindowCollectionBehaviorCanJoinAllSpaces | NSWindowCollectionBehaviorFullScreenAuxiliary;
    gGearPuck.releasedWhenClosed = NO;
    gGearPuck.contentView = [[BangGearView alloc] initWithFrame:NSMakeRect(0, 0, kPuckSize, kPuckSize)];
    bangPositionGear();

    // Floating pencil launcher, pinned to the left of the mic puck.
    gPencilPuck = [[NSPanel alloc]
        initWithContentRect:NSMakeRect(0, 0, kPuckSize, kPuckSize)
                  styleMask:(NSWindowStyleMaskBorderless | NSWindowStyleMaskNonactivatingPanel)
                    backing:NSBackingStoreBuffered
                      defer:NO];
    gPencilPuck.level = NSFloatingWindowLevel;
    gPencilPuck.opaque = NO;
    gPencilPuck.backgroundColor = [NSColor clearColor];
    gPencilPuck.hasShadow = YES;
    gPencilPuck.collectionBehavior =
        NSWindowCollectionBehaviorCanJoinAllSpaces | NSWindowCollectionBehaviorFullScreenAuxiliary;
    gPencilPuck.releasedWhenClosed = NO;
    gPencilPuck.contentView =
        [[BangPencilView alloc] initWithFrame:NSMakeRect(0, 0, kPuckSize, kPuckSize)];
    bangPositionPencil();
}

static BangPuckView *bangMicView(void) {
    return gMicPuck != nil ? (BangPuckView *)gMicPuck.contentView : nil;
}

static BangPuckView *bangSubmitView(void) {
    return gBoxSubmitView;
}

void bang_overlay_puck_show(void) {
    bangEnsurePucks();
    [gMicPuck orderFrontRegardless];
    [gGearPuck orderFrontRegardless];
    [gPencilPuck orderFrontRegardless];
}

void bang_overlay_puck_hide(void) {
    if (gMicPuck != nil) {
        [gMicPuck orderOut:nil];
    }
    if (gGearPuck != nil) {
        [gGearPuck orderOut:nil];
    }
    if (gPencilPuck != nil) {
        [gPencilPuck orderOut:nil];
    }
    bangSettingsPanelHide();
}

void bang_overlay_puck_set_listening(bool listening) {
    BangPuckView *view = bangMicView();
    if (view != nil) {
        view.listening = listening ? YES : NO;
        [view setNeedsDisplay:YES];
    }
}

void bang_overlay_puck_set_paused(bool paused) {
    BangPuckView *view = bangMicView();
    if (view != nil) {
        view.paused = paused ? YES : NO;
        [view setNeedsDisplay:YES];
    }
}

void bang_overlay_puck_set_level(double level) {
    BangPuckView *view = bangMicView();
    if (view != nil) {
        view.level = level;
        [view setNeedsDisplay:YES];
    }
}

void bang_overlay_puck_set_thinking(bool thinking) {
    BOOL on = thinking ? YES : NO;
    // Spin both the submit puck and the mic puck while the agent is working.
    BangPuckView *views[] = {bangSubmitView(), bangMicView()};
    for (int i = 0; i < 2; i++) {
        BangPuckView *view = views[i];
        if (view == nil) {
            continue;
        }
        view.thinking = on;
        if (on) {
            [view startSpin];
        } else {
            [view stopSpin];
        }
        [view setNeedsDisplay:YES];
    }
    // Shimmer the "Thinking…" text in the result box while working.
    bangBoxSetThinking(on);
}

// ------------------------------ Result box ------------------------------
// A 400x64 semi-transparent box above the pucks. Two modes:
//   - Dictation: editable, shows the live transcript (mirrors the composer).
//   - Result: read-only, streams the agent's thinking/result text.

/// A borderless panel that can still become key so its text view is editable.
@interface BangBoxPanel : NSPanel
@end
@implementation BangBoxPanel
- (BOOL)canBecomeKeyWindow {
    return YES;
}
@end

static BOOL gBoxEditing = NO;
// Set while we programmatically update the input view (see
// bang_overlay_box_set_input) so the resulting textDidChange isn't mistaken for a
// user edit — which would re-enter the Rust app context and crash.
static BOOL gBoxProgrammaticSet = NO;
// Box height model:
//   - expanded (default): auto-grow upward with content, capped only by the top
//     of the screen; then scroll.
//   - collapsed: force a single-line pill and scroll the content.
// The chevron toggles between the two; the grabber sets a manual height that
// overrides auto-fit until the chevron is used again.
static BOOL gBoxExpanded = YES;
// Manual height set by dragging the grabber (0 == auto-fit to content).
static CGFloat gBoxUserHeight = 0.0;
// Draggable handle at the top edge of the box (shrink/grow the height).
static NSView *gBoxGrabber = nil;
// Persisted settings mirrored onto the settings popover's switches. Stored at
// module scope so the switches can be initialized even if the panel is created
// after Rust first pushes these values.
static BOOL gAutoSubmitEnabled = NO;
static BOOL gVoiceEnabled = NO;
// Settings popover (toggled by the floating gear). The toggles are NSSwitch on
// macOS 10.15+, else a checkbox-style NSButton (both are NSControls with state).
static NSPanel *gSettingsPanel = nil;
static NSControl *gSettingsAutoSwitch = nil;
static NSControl *gSettingsVoiceSwitch = nil;

static void bangLayoutBox(void);

/// Forwards user edits in the box text view back to the composer.
@interface BangBoxDelegate : NSObject <NSTextViewDelegate>
@end
@implementation BangBoxDelegate
- (void)textDidBeginEditing:(NSNotification *)notification {
    (void)notification;
    gBoxEditing = YES;
}
// Enter submits (same path as the up-arrow puck); Shift+Enter inserts a newline.
- (BOOL)textView:(NSTextView *)textView doCommandBySelector:(SEL)commandSelector {
    if (commandSelector == @selector(insertNewline:)) {
        NSEvent *event = [NSApp currentEvent];
        if (event != nil && (event.modifierFlags & NSEventModifierFlagShift) != 0) {
            [textView insertNewlineIgnoringFieldEditor:nil];
            return YES;
        }
        bang_overlay_puck_clicked(BangPuckKindSubmit);
        return YES;
    }
    return NO;
}
- (void)textDidEndEditing:(NSNotification *)notification {
    (void)notification;
    gBoxEditing = NO;
}
- (void)textDidChange:(NSNotification *)notification {
    // Grow/shrink the box to fit, even for our own programmatic updates.
    bangLayoutBox();
    // Ignore changes we made ourselves (mirroring the transcript in).
    if (gBoxProgrammaticSet) {
        return;
    }
    NSTextView *tv = (NSTextView *)notification.object;
    NSString *snapshot = [tv.string copy];
    // Defer to the next runloop turn so we never call back into the Rust app
    // context while it's already mutably borrowed (AppKit may deliver this
    // synchronously inside an in-progress update).
    dispatch_async(dispatch_get_main_queue(), ^{
        const char *utf8 = snapshot.UTF8String;
        if (utf8 != NULL) {
            bang_overlay_box_edited(utf8);
        }
    });
}
- (void)toggleAutoSubmit:(id)sender {
    (void)sender;
    // Rust flips the persisted setting and pushes the new state back via
    // bang_overlay_box_set_auto_submit.
    bang_overlay_auto_submit_clicked();
}
- (void)toggleVoice:(id)sender {
    (void)sender;
    // Rust flips the persisted setting, stops any in-progress speech when turned
    // off, and pushes the new state back via bang_overlay_set_voice_enabled.
    bang_overlay_voice_toggled();
}
@end

/// The always-editable input line at the bottom of the box. acceptsFirstMouse so
/// a single click focuses it for typing even though the panel is non-activating.
@interface BangBoxTextView : NSTextView
@end
@implementation BangBoxTextView
- (BOOL)acceptsFirstMouse:(NSEvent *)event {
    (void)event;
    return YES;
}
@end

static NSPanel *gBox = nil;
// The editable input line (bottom band) — always editable; mirrors the composer.
static BangBoxTextView *gBoxInputView = nil;
static NSScrollView *gBoxInputScroll = nil;
// The read-only, scrollable conversation history (top region). Hidden until there
// is at least one exchange to show.
static NSTextView *gBoxHistoryView = nil;
static NSScrollView *gBoxHistoryScroll = nil;
// Thin separator between the history and the input line.
static NSView *gBoxDivider = nil;
static BangBoxDelegate *gBoxDelegate = nil;

// State for an in-progress group drag. Only one drag runs at a time (one mouse),
// so file-scope statics are sufficient.
static NSPoint gDragStartMouse;
static NSPoint gDragStartMicOrigin;
static NSPoint gDragStartBoxOrigin;
static BOOL gDragBoxWasVisible;

// Record the anchor positions when a drag begins on any overlay piece.
static void bangBeginGroupDrag(void) {
    gDragStartMouse = [NSEvent mouseLocation];
    gDragStartMicOrigin = (gMicPuck != nil) ? gMicPuck.frame.origin : NSZeroPoint;
    gDragBoxWasVisible = (gBox != nil && gBox.isVisible);
    gDragStartBoxOrigin = gDragBoxWasVisible ? gBox.frame.origin : NSZeroPoint;
}

// Translate the whole cluster by the mouse delta. The mic puck is the anchor;
// the gear + pencil are re-pinned relative to it, and the box (if it was visible
// when the drag started) rides along by the same delta.
static void bangUpdateGroupDrag(void) {
    if (gMicPuck == nil) {
        return;
    }
    NSPoint now = [NSEvent mouseLocation];
    CGFloat dx = now.x - gDragStartMouse.x;
    CGFloat dy = now.y - gDragStartMouse.y;
    [gMicPuck setFrameOrigin:NSMakePoint(gDragStartMicOrigin.x + dx, gDragStartMicOrigin.y + dy)];
    bangPositionGear();
    bangPositionPencil();
    if (gDragBoxWasVisible && gBox != nil) {
        [gBox setFrameOrigin:NSMakePoint(gDragStartBoxOrigin.x + dx, gDragStartBoxOrigin.y + dy)];
    }
}

// Height available for the box to grow upward: from its fixed bottom edge to a
// small margin below the top of the screen it lives on.
static CGFloat bangBoxMaxPanelHeight(void) {
    if (gBox == nil) {
        return 10000.0;
    }
    NSScreen *screen = gBox.screen ?: [NSScreen mainScreen];
    NSRect visible = (screen != nil) ? [screen visibleFrame] : NSMakeRect(0, 0, 1440, 900);
    CGFloat top = NSMaxY(visible) - 12.0; // leave a little breathing room
    CGFloat bottom = NSMinY(gBox.frame);
    CGFloat h = top - bottom;
    return (h < kPuckSize) ? kPuckSize : h;
}

// A thin draggable handle centered at the top edge of the box. Dragging it sets
// a manual height (gBoxUserHeight) so the user can shrink an auto-expanded box.
@interface BangGrabberView : NSView {
    NSTrackingArea *_trackingArea;
}
@end
@implementation BangGrabberView
- (void)drawRect:(NSRect)dirtyRect {
    (void)dirtyRect;
    NSRect b = self.bounds;
    CGFloat barW = 34.0, barH = 4.0;
    NSRect bar = NSMakeRect((b.size.width - barW) / 2.0, (b.size.height - barH) / 2.0, barW, barH);
    [[NSColor colorWithWhite:1.0 alpha:0.35] setFill];
    [[NSBezierPath bezierPathWithRoundedRect:bar xRadius:barH / 2.0 yRadius:barH / 2.0] fill];
}
- (void)resetCursorRects {
    [self addCursorRect:self.bounds cursor:[NSCursor resizeUpDownCursor]];
}
// `resetCursorRects` only fires while the owning window is key, but the overlay
// box is a non-activating panel that never becomes key — so the resize cursor
// never showed. Drive it explicitly via a tracking area that stays active
// regardless of app/window activation. `InVisibleRect` keeps it aligned with
// the view as `bangLayoutBox` repositions the grabber, without manual updates.
- (void)updateTrackingAreas {
    [super updateTrackingAreas];
    if (_trackingArea != nil) {
        [self removeTrackingArea:_trackingArea];
    }
    _trackingArea = [[NSTrackingArea alloc]
        initWithRect:self.bounds
             options:(NSTrackingMouseEnteredAndExited | NSTrackingActiveAlways |
                      NSTrackingInVisibleRect)
               owner:self
            userInfo:nil];
    [self addTrackingArea:_trackingArea];
}
- (void)mouseEntered:(NSEvent *)event {
    (void)event;
    [[NSCursor resizeUpDownCursor] set];
}
- (void)mouseExited:(NSEvent *)event {
    (void)event;
    [[NSCursor arrowCursor] set];
}
- (BOOL)acceptsFirstMouse:(NSEvent *)event {
    (void)event;
    return YES;
}
- (void)mouseDown:(NSEvent *)event {
    (void)event; // handled on drag
}
- (void)mouseDragged:(NSEvent *)event {
    (void)event;
    if (gBox == nil) {
        return;
    }
    NSPoint mouse = [NSEvent mouseLocation];
    CGFloat bottom = NSMinY(gBox.frame);
    CGFloat h = mouse.y - bottom; // distance from the fixed bottom edge
    CGFloat maxH = bangBoxMaxPanelHeight();
    if (h < kPuckSize) {
        h = kPuckSize;
    }
    if (h > maxH) {
        h = maxH;
    }
    // Dragging the grabber pins a manual height.
    gBoxUserHeight = h;
    gBoxExpanded = YES;
    bangLayoutBox();
}
@end

// A translucent light band that sweeps across the result box while the agent is
// working — a "spotlight" on the streaming Thinking…/answer text. Pointer events
// pass through (hitTest returns nil) so the text view stays interactive.
@interface BangBoxShimmerView : NSView
@property(nonatomic, assign) double phase;
@property(nonatomic, strong) NSTimer *timer;
@end
@implementation BangBoxShimmerView
- (NSView *)hitTest:(NSPoint)point {
    (void)point;
    return nil;
}
- (void)drawRect:(NSRect)dirtyRect {
    (void)dirtyRect;
    NSRect b = self.bounds;
    if (b.size.width <= 0.0) {
        return;
    }
    CGFloat bandW = b.size.width * 0.35;
    CGFloat t = fmod(self.phase, 1.0);
    CGFloat x = -bandW + t * (b.size.width + bandW);
    NSGradient *grad = [[NSGradient alloc] initWithColors:@[
        [NSColor colorWithWhite:1.0 alpha:0.0],
        [NSColor colorWithWhite:1.0 alpha:0.14],
        [NSColor colorWithWhite:1.0 alpha:0.0],
    ]];
    [grad drawInRect:NSMakeRect(x, 0, bandW, b.size.height) angle:0.0];
}
- (void)tick:(NSTimer *)timer {
    (void)timer;
    self.phase += 0.02;
    [self setNeedsDisplay:YES];
}
- (void)start {
    self.hidden = NO;
    if (self.timer != nil) {
        return;
    }
    self.phase = 0.0;
    self.timer = [NSTimer scheduledTimerWithTimeInterval:0.03
                                                  target:self
                                                selector:@selector(tick:)
                                                userInfo:nil
                                                 repeats:YES];
}
- (void)stop {
    [self.timer invalidate];
    self.timer = nil;
    self.hidden = YES;
    [self setNeedsDisplay:YES];
}
@end

static BangBoxShimmerView *gBoxShimmer = nil;

static void bangBoxSetThinking(BOOL on) {
    if (gBoxShimmer == nil) {
        return;
    }
    if (on) {
        [gBoxShimmer start];
    } else {
        [gBoxShimmer stop];
    }
}

static void bangEnsureBox(void) {
    if (gBox != nil) {
        return;
    }
    const CGFloat width = kBoxPillWidth;
    const CGFloat height = kPuckSize; // single-line pill minimum

    NSRect frame;
    if (gGearPuck != nil) {
        // Just to the right of the settings gear, bottom-aligned with the row.
        NSRect gear = gGearPuck.frame;
        frame = NSMakeRect(NSMaxX(gear) + kPuckGap, NSMinY(gear), width, height);
    } else {
        NSScreen *screen = [NSScreen mainScreen];
        NSRect visible = (screen != nil) ? [screen visibleFrame] : NSMakeRect(0, 0, 1440, 900);
        frame = NSMakeRect(NSMaxX(visible) - width - 24.0, NSMinY(visible) + 24.0, width, height);
    }

    BangBoxPanel *panel = [[BangBoxPanel alloc]
        initWithContentRect:frame
                  styleMask:(NSWindowStyleMaskBorderless | NSWindowStyleMaskNonactivatingPanel)
                    backing:NSBackingStoreBuffered
                      defer:NO];
    panel.level = NSFloatingWindowLevel;
    panel.opaque = NO;
    panel.backgroundColor = [NSColor clearColor];
    panel.hasShadow = YES;
    panel.collectionBehavior =
        NSWindowCollectionBehaviorCanJoinAllSpaces | NSWindowCollectionBehaviorFullScreenAuxiliary;
    panel.releasedWhenClosed = NO;

    NSView *bg = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, width, height)];
    bg.wantsLayer = YES;
    bg.layer.backgroundColor = [[NSColor colorWithWhite:0.0 alpha:0.5] CGColor];
    bg.layer.cornerRadius = height / 2.0; // full pill at min height
    bg.layer.masksToBounds = YES;

    // History region (top): read-only, scrollable, markdown-rendered transcript of
    // past exchanges + the in-flight one. Frames are set in bangLayoutBox; hidden
    // until there's history to show.
    NSScrollView *histScroll =
        [[NSScrollView alloc] initWithFrame:NSMakeRect(kBoxTextPadLeft, height, width - kBoxTextPadLeft - 2.0, 0)];
    histScroll.drawsBackground = NO;
    histScroll.hasVerticalScroller = YES;
    histScroll.autohidesScrollers = YES;
    histScroll.hidden = YES;
    NSTextView *hist = [[NSTextView alloc] initWithFrame:histScroll.bounds];
    hist.editable = NO;
    hist.selectable = YES;
    hist.drawsBackground = NO;
    hist.textColor = [NSColor whiteColor];
    hist.font = [NSFont systemFontOfSize:13.0];
    hist.textContainerInset = NSMakeSize(2.0, kBoxHistInsetV);
    hist.verticallyResizable = YES;
    hist.horizontallyResizable = NO;
    [hist.textContainer setWidthTracksTextView:YES];
    histScroll.documentView = hist;
    [bg addSubview:histScroll];
    gBoxHistoryScroll = histScroll;
    gBoxHistoryView = hist;

    // Divider between history and input. Hidden with the history region.
    NSView *divider = [[NSView alloc] initWithFrame:NSMakeRect(kBoxTextPadLeft, height, width - 2.0 * kBoxTextPadLeft, kBoxDividerH)];
    divider.wantsLayer = YES;
    divider.layer.backgroundColor = [[NSColor colorWithWhite:1.0 alpha:0.14] CGColor];
    divider.hidden = YES;
    [bg addSubview:divider];
    gBoxDivider = divider;

    // Input line (bottom band): always editable; mirrors the composer. Extends to
    // the far right so the vertical scroller hugs the right side. The vertical inset
    // centers a single line in the pill band.
    NSRect scrollFrame =
        NSMakeRect(kBoxTextPadLeft, 0, width - kBoxTextPadLeft - 2.0, height);
    NSScrollView *scroll = [[NSScrollView alloc] initWithFrame:scrollFrame];
    scroll.drawsBackground = NO;
    scroll.hasVerticalScroller = YES;
    scroll.autohidesScrollers = YES;

    BangBoxTextView *tv = [[BangBoxTextView alloc] initWithFrame:scroll.bounds];
    tv.editable = YES;
    tv.selectable = YES;
    tv.drawsBackground = NO;
    tv.textColor = [NSColor whiteColor];
    tv.insertionPointColor = [NSColor whiteColor];
    tv.font = [NSFont systemFontOfSize:13.0];
    tv.textContainerInset = NSMakeSize(2.0, kBoxTextInsetV);
    tv.verticallyResizable = YES;
    tv.horizontallyResizable = NO;
    [tv.textContainer setWidthTracksTextView:YES];
    gBoxDelegate = [[BangBoxDelegate alloc] init];
    tv.delegate = gBoxDelegate;
    scroll.documentView = tv;

    [bg addSubview:scroll];
    gBoxInputScroll = scroll;

    // Submit button: a small up-arrow puck vertically centered in the pill's
    // bottom (single-line) band, on the right. Reuses BangPuckView (kind submit).
    // Embedded so it never drags the box. Sticks to the bottom-right as the pill
    // grows upward.
    BangPuckView *submit = [[BangPuckView alloc]
        initWithFrame:NSMakeRect(width - kBoxSubmitPad - kBoxSubmitSize,
                                 (kPuckSize - kBoxSubmitSize) / 2.0, kBoxSubmitSize, kBoxSubmitSize)];
    submit.kind = BangPuckKindSubmit;
    submit.embedded = YES;
    submit.autoresizingMask = NSViewMinXMargin | NSViewMaxYMargin;
    // In auto-submit mode there's nothing to press, so hide the send button.
    submit.hidden = gAutoSubmitEnabled;
    [bg addSubview:submit];
    gBoxSubmitView = submit;

    // Grabber: a thin handle centered at the top edge. Stays pinned to the top as
    // the box grows upward; hidden while the box is a single-line pill.
    const CGFloat grabberH = 12.0;
    BangGrabberView *grabber =
        [[BangGrabberView alloc] initWithFrame:NSMakeRect(0, height - grabberH, width, grabberH)];
    grabber.autoresizingMask = NSViewWidthSizable | NSViewMinYMargin;
    grabber.hidden = YES;
    [bg addSubview:grabber];
    gBoxGrabber = grabber;

    // "Thinking" spotlight shimmer, on top of everything but click-through. Fills
    // the box and follows its height; hidden until the agent is working.
    BangBoxShimmerView *shimmer =
        [[BangBoxShimmerView alloc] initWithFrame:NSMakeRect(0, 0, width, height)];
    shimmer.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
    shimmer.hidden = YES;
    [bg addSubview:shimmer];
    gBoxShimmer = shimmer;

    panel.contentView = bg;
    gBox = panel;
    gBoxInputView = tv;
    bangLayoutBox();
}

// Two-region layout: an always-editable input line (bottom band) and, above it, a
// read-only scrollable history of the conversation. The bottom edge stays fixed so
// the box grows upward; when there's no history it's just the input pill.
//
// - Input band: auto-fits its content up to a few lines, then scrolls.
// - History region: fills the space between the input band and the top of the
//   screen (auto-fit to content, capped), then scrolls. A manual grabber height
//   (gBoxUserHeight = total panel height) overrides the auto-fit; the collapse
//   state (gBoxExpanded == NO) hides the history entirely.
static void bangLayoutBox(void) {
    if (gBox == nil || gBoxInputView == nil) {
        return;
    }
    NSFont *font = gBoxInputView.font ?: [NSFont systemFontOfSize:13.0];
    NSLayoutManager *probe = [[NSLayoutManager alloc] init];
    CGFloat lineH = [probe defaultLineHeightForFont:font];
    if (lineH <= 0.0) {
        lineH = 16.0;
    }

    // --- Input band height (auto-fit to content, capped at ~5 lines) ---
    [gBoxInputView.layoutManager ensureLayoutForTextContainer:gBoxInputView.textContainer];
    CGFloat inputContentH =
        [gBoxInputView.layoutManager usedRectForTextContainer:gBoxInputView.textContainer].size.height;
    if (inputContentH < lineH) {
        inputContentH = lineH;
    }
    CGFloat inputH = ceil(inputContentH + 2.0 * kBoxTextInsetV);
    if (inputH < kPuckSize) {
        inputH = kPuckSize;
    }
    CGFloat inputMaxH = kPuckSize + 4.0 * lineH;
    if (inputH > inputMaxH) {
        inputH = inputMaxH;
    }

    CGFloat maxPanelH = bangBoxMaxPanelHeight();

    // --- History region height ---
    BOOL hasHistory = (gBoxHistoryView != nil && gBoxHistoryView.string.length > 0);
    CGFloat dividerH = hasHistory ? kBoxDividerH : 0.0;
    CGFloat historyH = 0.0;
    if (hasHistory && gBoxExpanded) {
        [gBoxHistoryView.layoutManager ensureLayoutForTextContainer:gBoxHistoryView.textContainer];
        CGFloat histContentH =
            [gBoxHistoryView.layoutManager usedRectForTextContainer:gBoxHistoryView.textContainer]
                .size.height;
        CGFloat wantH = ceil(histContentH + 2.0 * kBoxHistInsetV);
        CGFloat availH = maxPanelH - inputH - dividerH;
        if (gBoxUserHeight > 0.0) {
            // Manual total height: give the remainder to history.
            historyH = gBoxUserHeight - inputH - dividerH;
        } else {
            historyH = wantH;
        }
        if (historyH > availH) {
            historyH = availH;
        }
        if (historyH < lineH) {
            historyH = lineH;
        }
    }
    BOOL showHistory = hasHistory && gBoxExpanded && historyH > 0.0;

    CGFloat panelH = inputH + (showHistory ? historyH + dividerH : 0.0);
    if (panelH < kPuckSize) {
        panelH = kPuckSize;
    }
    if (panelH > maxPanelH) {
        panelH = maxPanelH;
        // Recompute history to fit within the cap.
        if (showHistory) {
            historyH = panelH - inputH - dividerH;
            if (historyH < lineH) {
                historyH = lineH;
            }
        }
    }

    // Full pill only when it's a single-line input with no history; else a softer
    // rounded rect.
    CGFloat corner =
        (!showHistory && panelH <= kPuckSize + 0.5) ? panelH / 2.0 : 16.0;
    NSView *bg = gBox.contentView;
    if (bg != nil && bg.layer != nil) {
        bg.layer.cornerRadius = corner;
    }

    NSRect f = gBox.frame;
    if (fabs(f.size.height - panelH) >= 0.5) {
        // Keep origin.y (the bottom edge) fixed so the box grows/shrinks upward.
        f.size.height = panelH;
        [gBox setFrame:f display:YES];
    }

    const CGFloat width = f.size.width;
    // Input band pinned to the bottom.
    if (gBoxInputScroll != nil) {
        gBoxInputScroll.frame =
            NSMakeRect(kBoxTextPadLeft, 0, width - kBoxTextPadLeft - 2.0, inputH);
    }
    // History + divider stacked above the input band.
    if (gBoxDivider != nil) {
        gBoxDivider.hidden = !showHistory;
        gBoxDivider.frame = NSMakeRect(kBoxTextPadLeft, inputH, width - 2.0 * kBoxTextPadLeft, kBoxDividerH);
    }
    if (gBoxHistoryScroll != nil) {
        gBoxHistoryScroll.hidden = !showHistory;
        if (showHistory) {
            gBoxHistoryScroll.frame =
                NSMakeRect(kBoxTextPadLeft, inputH + dividerH, width - kBoxTextPadLeft - 2.0, historyH);
        }
    }

    // The grabber only makes sense when the history is showing (something to shrink).
    if (gBoxGrabber != nil) {
        gBoxGrabber.hidden = !showHistory;
        gBoxGrabber.frame = NSMakeRect(0, panelH - 12.0, width, 12.0);
    }

    [gBoxInputView scrollRangeToVisible:NSMakeRange((NSInteger)gBoxInputView.string.length, 0)];
    if (showHistory) {
        [gBoxHistoryView scrollRangeToVisible:NSMakeRange((NSInteger)gBoxHistoryView.string.length, 0)];
    }
}

void bang_overlay_box_show(void) {
    bangEnsureBox();
    bangLayoutBox();
    [gBox orderFrontRegardless];
}

void bang_overlay_box_hide(void) {
    if (gBox != nil) {
        [gBox orderOut:nil];
    }
}

// ------------------------------ Settings popover ------------------------------
// A small floating panel with switches for "Auto-submit" and "Read answers
// aloud", toggled by the floating gear. Each switch flips the persisted setting
// via the same Rust entrypoints the old menu used.

@interface BangSettingsDelegate : NSObject
@end
@implementation BangSettingsDelegate
- (void)toggleAutoSubmit:(id)sender {
    (void)sender;
    bang_overlay_auto_submit_clicked();
}
- (void)toggleVoice:(id)sender {
    (void)sender;
    bang_overlay_voice_toggled();
}
- (void)selectLanguage:(id)sender {
    NSPopUpButton *popup = (NSPopUpButton *)sender;
    NSInteger idx = [popup indexOfSelectedItem];
    if (idx >= 0 && idx < kLangOptionCount) {
        bang_overlay_language_selected(kLangOptions[idx].code);
    }
}
- (void)changeVerbosity:(id)sender {
    NSSlider *slider = (NSSlider *)sender;
    int level = (int)lround(slider.doubleValue);
    if (level < 0) {
        level = 0;
    } else if (level > 10) {
        level = 10;
    }
    gVerbosity = level;
    bangUpdateVerbosityLabel();
    bang_overlay_verbosity_selected(level);
}
@end

static BangSettingsDelegate *gSettingsDelegate = nil;

// A round color swatch in the settings popover. Clicking it selects that preset
// (routed through Rust as bang_overlay_puck_color_clicked, which persists it and
// pushes the new index back via bang_overlay_set_puck_color).
@interface BangSwatchView : NSView
@property(nonatomic, assign) NSInteger index;
@property(nonatomic, assign) BOOL selected;
@end
@implementation BangSwatchView
- (BOOL)acceptsFirstMouse:(NSEvent *)event {
    (void)event;
    return YES;
}
- (void)drawRect:(NSRect)dirtyRect {
    (void)dirtyRect;
    NSRect fill = NSInsetRect(self.bounds, 2.0, 2.0);
    NSBezierPath *oval = [NSBezierPath bezierPathWithOvalInRect:fill];
    // Render the preset itself (solid or gradient) so the swatch previews it.
    bangFillOvalPreset(oval, self.index, NO);
    // A white selection ring marks the active preset.
    NSBezierPath *ring = [NSBezierPath
        bezierPathWithOvalInRect:NSInsetRect(self.bounds, 0.5, 0.5)];
    ring.lineWidth = self.selected ? 2.0 : 1.0;
    [[NSColor colorWithWhite:1.0 alpha:self.selected ? 1.0 : 0.25] setStroke];
    [ring stroke];
}
- (void)mouseDown:(NSEvent *)event {
    (void)event;
}
- (void)mouseUp:(NSEvent *)event {
    (void)event;
    bang_overlay_puck_color_clicked((int)self.index);
}
@end

// The swatch views, kept so bang_overlay_set_puck_color can refresh the
// selection ring when the color changes.
static NSMutableArray<BangSwatchView *> *gSwatchViews = nil;
// The language popup, kept so bang_overlay_set_language can reflect the current
// selection when it changes.
static NSPopUpButton *gLangPopup = nil;

// Select the popup item matching an ISO code (empty = auto-detect).
static void bangSelectLangItem(const char *code) {
    if (gLangPopup == nil) {
        return;
    }
    const char *want = code ? code : "";
    for (int i = 0; i < kLangOptionCount; i++) {
        if (strcmp(kLangOptions[i].code, want) == 0) {
            [gLangPopup selectItemAtIndex:i];
            return;
        }
    }
}

// The verbosity slider + its numeric readout, kept so bang_overlay_set_verbosity
// can reflect an externally-changed value (e.g. from Settings > AI).
static NSSlider *gVerbositySlider = nil;
static NSTextField *gVerbosityValueLabel = nil;

// Refresh the "N" readout beside the verbosity slider from gVerbosity.
static void bangUpdateVerbosityLabel(void) {
    if (gVerbosityValueLabel != nil) {
        gVerbosityValueLabel.stringValue = [NSString stringWithFormat:@"%d", gVerbosity];
    }
}

// A toggle switch (NSSwitch on 10.15+, checkbox NSButton otherwise).
static NSControl *bangMakeToggle(NSRect frame, id target, SEL action, BOOL on) {
    if (@available(macOS 10.15, *)) {
        NSSwitch *sw = [[NSSwitch alloc] initWithFrame:frame];
        sw.target = target;
        sw.action = action;
        sw.state = on ? NSControlStateValueOn : NSControlStateValueOff;
        return sw;
    }
    NSButton *btn = [[NSButton alloc] initWithFrame:frame];
    [btn setButtonType:NSButtonTypeSwitch];
    btn.title = @"";
    btn.target = target;
    btn.action = action;
    btn.state = on ? NSControlStateValueOn : NSControlStateValueOff;
    return btn;
}

static NSTextField *bangSettingsLabel(NSString *title, NSRect frame) {
    NSTextField *label = [[NSTextField alloc] initWithFrame:frame];
    label.stringValue = title;
    label.editable = NO;
    label.selectable = NO;
    label.bordered = NO;
    label.drawsBackground = NO;
    label.textColor = [NSColor whiteColor];
    label.font = [NSFont systemFontOfSize:12.0];
    return label;
}

static void bangEnsureSettingsPanel(void) {
    if (gSettingsPanel != nil) {
        return;
    }
    const CGFloat width = 230.0;
    const CGFloat pad = 10.0;
    const CGFloat rowH = 26.0;
    const CGFloat rowGap = 6.0;
    const CGFloat switchW = 40.0;
    const CGFloat switchH = 20.0;
    // Rows: Auto-submit, Read aloud, Color, Gradient, Language, Verbosity.
    const CGFloat height = 2.0 * pad + 6.0 * rowH + 5.0 * rowGap;

    gSettingsDelegate = [[BangSettingsDelegate alloc] init];

    NSPanel *panel = [[NSPanel alloc]
        initWithContentRect:NSMakeRect(0, 0, width, height)
                  styleMask:(NSWindowStyleMaskBorderless | NSWindowStyleMaskNonactivatingPanel)
                    backing:NSBackingStoreBuffered
                      defer:NO];
    panel.level = NSFloatingWindowLevel;
    panel.opaque = NO;
    panel.backgroundColor = [NSColor clearColor];
    panel.hasShadow = YES;
    panel.collectionBehavior =
        NSWindowCollectionBehaviorCanJoinAllSpaces | NSWindowCollectionBehaviorFullScreenAuxiliary;
    panel.releasedWhenClosed = NO;

    NSView *bg = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, width, height)];
    bg.wantsLayer = YES;
    bg.layer.backgroundColor = [[NSColor colorWithWhite:0.0 alpha:0.85] CGColor];
    bg.layer.cornerRadius = 10.0;
    bg.layer.masksToBounds = YES;

    // Evenly-stacked rows from the top: Auto-submit, Read aloud, Color, Gradient,
    // Language.
    CGFloat autoRowY = height - pad - rowH;
    CGFloat voiceRowY = autoRowY - rowH - rowGap;
    CGFloat colorRowY = voiceRowY - rowH - rowGap;
    CGFloat gradientRowY = colorRowY - rowH - rowGap;
    CGFloat langRowY = gradientRowY - rowH - rowGap;
    CGFloat verbosityRowY = langRowY - rowH - rowGap;
    CGFloat switchX = width - pad - switchW;

    // Auto-submit (top row).
    [bg addSubview:bangSettingsLabel(@"Auto-submit",
                                     NSMakeRect(12.0, autoRowY + 4.0, 120.0, 18.0))];
    NSControl *autoSwitch = bangMakeToggle(
        NSMakeRect(switchX, autoRowY + (rowH - switchH) / 2.0, switchW, switchH),
        gSettingsDelegate, @selector(toggleAutoSubmit:), gAutoSubmitEnabled);
    [bg addSubview:autoSwitch];
    gSettingsAutoSwitch = autoSwitch;

    // Read answers aloud (middle row).
    [bg addSubview:bangSettingsLabel(@"Read answers aloud",
                                     NSMakeRect(12.0, voiceRowY + 4.0, 150.0, 18.0))];
    NSControl *voiceSwitch = bangMakeToggle(
        NSMakeRect(switchX, voiceRowY + (rowH - switchH) / 2.0, switchW, switchH),
        gSettingsDelegate, @selector(toggleVoice:), gVoiceEnabled);
    [bg addSubview:voiceSwitch];
    gSettingsVoiceSwitch = voiceSwitch;

    // Color + Gradient rows: a label plus right-aligned preset swatches. Solid
    // presets fill the Color row; gradient presets fill the Gradient row.
    [bg addSubview:bangSettingsLabel(@"Color",
                                     NSMakeRect(12.0, colorRowY + 4.0, 60.0, 18.0))];
    [bg addSubview:bangSettingsLabel(@"Gradient",
                                     NSMakeRect(12.0, gradientRowY + 4.0, 80.0, 18.0))];
    const CGFloat sw = 20.0;
    const CGFloat swGap = 8.0;
    NSInteger solidCount = 0, gradCount = 0;
    for (NSInteger i = 0; i < kPuckPresetCount; i++) {
        if (kPuckPresets[i].gradient) {
            gradCount++;
        } else {
            solidCount++;
        }
    }
    CGFloat solidStartX = width - pad - (solidCount * sw + (solidCount - 1) * swGap);
    CGFloat gradStartX = width - pad - (gradCount * sw + (gradCount - 1) * swGap);
    CGFloat solidY = colorRowY + (rowH - sw) / 2.0;
    CGFloat gradY = gradientRowY + (rowH - sw) / 2.0;
    gSwatchViews = [NSMutableArray array];
    NSInteger solidCol = 0, gradCol = 0;
    for (NSInteger i = 0; i < kPuckPresetCount; i++) {
        BangPuckPreset p = kPuckPresets[i];
        NSRect frame;
        if (p.gradient) {
            frame = NSMakeRect(gradStartX + gradCol * (sw + swGap), gradY, sw, sw);
            gradCol++;
        } else {
            frame = NSMakeRect(solidStartX + solidCol * (sw + swGap), solidY, sw, sw);
            solidCol++;
        }
        BangSwatchView *swatch = [[BangSwatchView alloc] initWithFrame:frame];
        swatch.index = i;
        swatch.selected = (i == gPuckColorIndex);
        [bg addSubview:swatch];
        [gSwatchViews addObject:swatch];
    }

    // Language (bottom row): label + popup of transcription languages.
    [bg addSubview:bangSettingsLabel(@"Language",
                                     NSMakeRect(12.0, langRowY + 4.0, 80.0, 18.0))];
    const CGFloat popupW = 120.0;
    const CGFloat popupH = 22.0;
    NSPopUpButton *popup = [[NSPopUpButton alloc]
        initWithFrame:NSMakeRect(width - pad - popupW, langRowY + (rowH - popupH) / 2.0, popupW,
                                 popupH)
            pullsDown:NO];
    for (int i = 0; i < kLangOptionCount; i++) {
        [popup addItemWithTitle:[NSString stringWithUTF8String:kLangOptions[i].name]];
    }
    popup.target = gSettingsDelegate;
    popup.action = @selector(selectLanguage:);
    [bg addSubview:popup];
    gLangPopup = popup;
    bangSelectLangItem(gLangCode);

    // Verbosity (bottom row): label + 0-10 slider + numeric readout. Mirrors the
    // "Response verbosity" slider in Settings > AI (same persisted setting).
    [bg addSubview:bangSettingsLabel(@"Verbosity",
                                     NSMakeRect(12.0, verbosityRowY + 4.0, 70.0, 18.0))];
    const CGFloat valueW = 18.0;
    const CGFloat sliderH = 20.0;
    const CGFloat sliderX = 12.0 + 70.0;
    const CGFloat sliderW = width - pad - valueW - 6.0 - sliderX;
    NSSlider *vSlider = [[NSSlider alloc]
        initWithFrame:NSMakeRect(sliderX, verbosityRowY + (rowH - sliderH) / 2.0, sliderW, sliderH)];
    vSlider.minValue = 0.0;
    vSlider.maxValue = 10.0;
    vSlider.numberOfTickMarks = 11;
    vSlider.allowsTickMarkValuesOnly = YES;
    vSlider.doubleValue = (double)gVerbosity;
    vSlider.target = gSettingsDelegate;
    vSlider.action = @selector(changeVerbosity:);
    [bg addSubview:vSlider];
    gVerbositySlider = vSlider;

    NSTextField *vValue = bangSettingsLabel(
        @"", NSMakeRect(width - pad - valueW, verbosityRowY + 4.0, valueW, 18.0));
    vValue.alignment = NSTextAlignmentRight;
    [bg addSubview:vValue];
    gVerbosityValueLabel = vValue;
    bangUpdateVerbosityLabel();

    panel.contentView = bg;
    gSettingsPanel = panel;
}

// Position the settings popover just above the gear, clamped on-screen.
static void bangPositionSettingsPanel(void) {
    if (gSettingsPanel == nil || gGearPuck == nil) {
        return;
    }
    NSRect gear = gGearPuck.frame;
    NSRect panel = gSettingsPanel.frame;
    CGFloat x = NSMaxX(gear) - panel.size.width;
    CGFloat y = NSMaxY(gear) + 8.0;
    NSScreen *screen = gGearPuck.screen ?: [NSScreen mainScreen];
    if (screen != nil) {
        NSRect visible = [screen visibleFrame];
        if (x < NSMinX(visible)) {
            x = NSMinX(visible) + 4.0;
        }
        if (NSMaxX(NSMakeRect(x, y, panel.size.width, panel.size.height)) > NSMaxX(visible)) {
            x = NSMaxX(visible) - panel.size.width - 4.0;
        }
        if (y + panel.size.height > NSMaxY(visible)) {
            y = NSMinY(gear) - panel.size.height - 8.0;
        }
    }
    [gSettingsPanel setFrameOrigin:NSMakePoint(x, y)];
}

// Mouse-down monitors installed while the settings popover is open so it can
// close on blur. The panel is a non-activating borderless NSPanel (never key),
// so we can't rely on windowDidResignKey — instead we watch for clicks outside
// the panel. The global monitor catches clicks in other apps; the local monitor
// catches clicks elsewhere in our own overlay (box, other pucks, empty space).
static id gSettingsGlobalMonitor = nil;
static id gSettingsLocalMonitor = nil;

static void bangSettingsRemoveBlurMonitors(void) {
    if (gSettingsGlobalMonitor != nil) {
        [NSEvent removeMonitor:gSettingsGlobalMonitor];
        gSettingsGlobalMonitor = nil;
    }
    if (gSettingsLocalMonitor != nil) {
        [NSEvent removeMonitor:gSettingsLocalMonitor];
        gSettingsLocalMonitor = nil;
    }
}

static void bangSettingsInstallBlurMonitors(void) {
    NSEventMask mask =
        NSEventMaskLeftMouseDown | NSEventMaskRightMouseDown | NSEventMaskOtherMouseDown;
    if (gSettingsGlobalMonitor == nil) {
        gSettingsGlobalMonitor =
            [NSEvent addGlobalMonitorForEventsMatchingMask:mask
                                                   handler:^(NSEvent *event) {
                                                     (void)event;
                                                     bangSettingsPanelHide();
                                                   }];
    }
    if (gSettingsLocalMonitor == nil) {
        gSettingsLocalMonitor = [NSEvent
            addLocalMonitorForEventsMatchingMask:mask
                                         handler:^NSEvent *(NSEvent *event) {
                                           // Keep the popover open for clicks on
                                           // itself or on the gear (whose own
                                           // handler toggles it closed).
                                           NSWindow *w = event.window;
                                           if (w != gSettingsPanel && w != (NSWindow *)gGearPuck) {
                                               bangSettingsPanelHide();
                                           }
                                           return event;
                                         }];
    }
}

static void bangToggleSettingsPanel(void) {
    bangEnsureSettingsPanel();
    if (gSettingsPanel.isVisible) {
        bangSettingsPanelHide();
    } else {
        bangPositionSettingsPanel();
        [gSettingsPanel orderFrontRegardless];
        bangSettingsInstallBlurMonitors();
    }
}

static void bangSettingsPanelHide(void) {
    bangSettingsRemoveBlurMonitors();
    if (gSettingsPanel != nil) {
        [gSettingsPanel orderOut:nil];
    }
}

// Raw source of the last text set programmatically for each region, so streaming
// updates can be deduped even though the history stores an attributed
// (markdown-rendered) copy whose `.string` differs from the source.
static NSString *gBoxHistoryLastRaw = nil;
static NSString *gBoxInputLastRaw = nil;

// Renders markdown (bold, code, italics, links) to an attributed string in white
// so the result box inherits the main UI's formatting. Falls back to plain text
// pre-macOS 12.
static NSAttributedString *bangRenderMarkdown(NSString *text) {
    NSFont *baseFont = [NSFont systemFontOfSize:11.0];
    NSMutableAttributedString *result = nil;
    if (@available(macOS 12.0, *)) {
        NSAttributedStringMarkdownParsingOptions *opts =
            [[NSAttributedStringMarkdownParsingOptions alloc] init];
        // Keep line breaks; render inline bold/code/etc.
        opts.interpretedSyntax =
            NSAttributedStringMarkdownInterpretedSyntaxInlineOnlyPreservingWhitespace;
        NSError *error = nil;
        NSAttributedString *parsed = [[NSAttributedString alloc] initWithMarkdownString:text
                                                                                options:opts
                                                                                baseURL:nil
                                                                                  error:&error];
        if (parsed != nil) {
            result = [parsed mutableCopy];
        }
    }
    if (result == nil) {
        result = [[NSMutableAttributedString alloc] initWithString:text];
    }
    NSRange full = NSMakeRange(0, result.length);
    [result addAttribute:NSForegroundColorAttributeName value:[NSColor whiteColor] range:full];
    // Fill in a base font wherever markdown parsing didn't set one.
    [result enumerateAttribute:NSFontAttributeName
                       inRange:full
                       options:0
                    usingBlock:^(id value, NSRange range, BOOL *stop) {
                      (void)stop;
                      if (value == nil) {
                          [result addAttribute:NSFontAttributeName value:baseFont range:range];
                      }
                    }];

    // User-prompt lines arrive prefixed with "> " (a machine marker from the
    // Rust side, left as literal text by the inline-only markdown parse). Strip
    // that marker and give the line a subtle lighter-grey background so the
    // user's question reads as a distinct bubble. Iterate the original line
    // ranges back-to-front so earlier locations stay valid as we delete the
    // "> " prefixes.
    NSColor *userBg = [NSColor colorWithWhite:1.0 alpha:0.12];
    NSString *source = result.string;
    NSMutableArray<NSValue *> *lineRanges = [NSMutableArray array];
    [source enumerateSubstringsInRange:NSMakeRange(0, source.length)
                               options:NSStringEnumerationByLines |
                                       NSStringEnumerationSubstringNotRequired
                            usingBlock:^(NSString *sub, NSRange lineRange, NSRange enclosing,
                                         BOOL *stop) {
                              (void)sub;
                              (void)enclosing;
                              (void)stop;
                              [lineRanges addObject:[NSValue valueWithRange:lineRange]];
                            }];
    for (NSInteger i = (NSInteger)lineRanges.count - 1; i >= 0; i--) {
        NSRange r = [lineRanges[(NSUInteger)i] rangeValue];
        if (r.length < 2) {
            continue;
        }
        if ([[source substringWithRange:NSMakeRange(r.location, 2)] isEqualToString:@"> "]) {
            [result deleteCharactersInRange:NSMakeRange(r.location, 2)];
            [result addAttribute:NSBackgroundColorAttributeName
                           value:userBg
                           range:NSMakeRange(r.location, r.length - 2)];
        }
    }
    return result;
}

// Set the read-only conversation history (markdown). Passing "" clears + hides it.
void bang_overlay_box_set_history(const char *utf8) {
    if (gBoxHistoryView == nil || utf8 == NULL) {
        return;
    }
    NSString *text = [NSString stringWithUTF8String:utf8];
    if (text == nil) {
        return;
    }
    if (gBoxHistoryLastRaw != nil && [text isEqualToString:gBoxHistoryLastRaw]) {
        return;
    }
    gBoxHistoryLastRaw = [text copy];
    if (text.length == 0) {
        [gBoxHistoryView.textStorage setAttributedString:[[NSAttributedString alloc] initWithString:@""]];
    } else {
        [gBoxHistoryView.textStorage setAttributedString:bangRenderMarkdown(text)];
    }
    bangLayoutBox();
    [gBoxHistoryView scrollRangeToVisible:NSMakeRange((NSInteger)gBoxHistoryView.string.length, 0)];
}

// Set the editable input line (plain text). Skips while the user is actively
// typing so a streaming transcript/mirror never clobbers an in-progress edit.
void bang_overlay_box_set_input(const char *utf8) {
    if (gBoxInputView == nil || utf8 == NULL) {
        return;
    }
    if (gBoxEditing) {
        return;
    }
    NSString *text = [NSString stringWithUTF8String:utf8];
    if (text == nil) {
        return;
    }
    if (gBoxInputLastRaw != nil && [text isEqualToString:gBoxInputLastRaw]) {
        return;
    }
    gBoxInputLastRaw = [text copy];
    gBoxProgrammaticSet = YES;
    gBoxInputView.string = text;
    gBoxProgrammaticSet = NO;
    bangLayoutBox();
    [gBoxInputView scrollRangeToVisible:NSMakeRange((NSInteger)gBoxInputView.string.length, 0)];
}

// Reflect the persisted auto-submit setting on the settings popover switch.
void bang_overlay_box_set_auto_submit(bool on) {
    gAutoSubmitEnabled = on ? YES : NO;
    // integerValue maps to state for both NSSwitch and the checkbox fallback.
    gSettingsAutoSwitch.integerValue = on ? 1 : 0;
    // In auto-submit mode there's nothing to press, so hide the send button.
    gBoxSubmitView.hidden = gAutoSubmitEnabled;
}

// Reflect the persisted "read answers aloud" setting on the settings popover.
void bang_overlay_set_voice_enabled(bool on) {
    gVoiceEnabled = on ? YES : NO;
    gSettingsVoiceSwitch.integerValue = on ? 1 : 0;
}

// Set the current transcription language (ISO code, "" = auto-detect). Stores it
// for when the panel is built and reflects it in the popup if already built.
void bang_overlay_set_language(const char *code) {
    const char *c = code ? code : "";
    snprintf(gLangCode, sizeof(gLangCode), "%s", c);
    bangSelectLangItem(gLangCode);
}

// Set the current response-verbosity level (0-10). Stores it for when the panel
// is built and reflects it on the slider/readout if already built.
void bang_overlay_set_verbosity(int level) {
    if (level < 0) {
        level = 0;
    } else if (level > 10) {
        level = 10;
    }
    gVerbosity = level;
    if (gVerbositySlider != nil) {
        gVerbositySlider.doubleValue = (double)level;
    }
    bangUpdateVerbosityLabel();
}

// Set the shared puck accent color to a preset index (see kPuckPresets). Redraws
// all pucks and refreshes the swatch selection ring. Nil-messaging makes the
// content-view redraws safe even before the pucks/panel exist.
void bang_overlay_set_puck_color(int index) {
    gPuckColorIndex = index;
    if (gPuckColorIndex < 0 || gPuckColorIndex >= kPuckPresetCount) {
        gPuckColorIndex = 0;
    }
    [gMicPuck.contentView setNeedsDisplay:YES];
    [gGearPuck.contentView setNeedsDisplay:YES];
    [gPencilPuck.contentView setNeedsDisplay:YES];
    [gBoxSubmitView setNeedsDisplay:YES];
    for (BangSwatchView *swatch in gSwatchViews) {
        swatch.selected = (swatch.index == gPuckColorIndex);
        [swatch setNeedsDisplay:YES];
    }
}

// Set the box background color (RGBA components in 0..1) so it can match the
// main composer's background.
void bang_overlay_box_set_bg(double r, double g, double b, double a) {
    if (gBox == nil) {
        return;
    }
    NSView *bg = gBox.contentView;
    if (bg != nil) {
        bg.wantsLayer = YES;
        (void)a; // fixed 50% transparency regardless of the composer's alpha
        bg.layer.backgroundColor =
            [[NSColor colorWithSRGBRed:r green:g blue:b alpha:0.5] CGColor];
    }
}

// ------------------------------ Annotation canvas ------------------------------
// A full-screen, transparent, always-on-top panel for the pencil flow. The user
// draws freeform strokes; the window their FIRST stroke lands on is the "picked"
// window (found via CGWindowList) and is outlined. On Done we hide the chrome
// (outline + buttons), keep the strokes on screen, and hand the picked window's
// screen rect back to Rust, which runs `screencapture -R` so the PNG contains the
// window plus the strokes (no manual compositing). Cancel discards everything.

// Bright pink used for strokes + the picked-window outline (matches the puck).
#define BANG_INK [NSColor colorWithSRGBRed:1.0 green:0.176 blue:0.471 alpha:1.0]

// Frontmost normal window under `p` (CoreGraphics global point, top-left origin),
// excluding our own app. Returns its bounds (CG global, top-left, points) and the
// owner process id (so we can raise that app to the front).
static BOOL bangWindowUnderPoint(CGPoint p, CGRect *out, pid_t *outPid) {
    CFArrayRef list = CGWindowListCopyWindowInfo(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements, kCGNullWindowID);
    if (list == NULL) {
        return NO;
    }
    BOOL found = NO;
    CFIndex count = CFArrayGetCount(list);
    for (CFIndex i = 0; i < count; i++) {
        CFDictionaryRef info = (CFDictionaryRef)CFArrayGetValueAtIndex(list, i);
        CFNumberRef layerRef = (CFNumberRef)CFDictionaryGetValue(info, kCGWindowLayer);
        int layer = 0;
        if (layerRef != NULL) {
            CFNumberGetValue(layerRef, kCFNumberIntType, &layer);
        }
        // Only pick normal-level windows (layer 0). This also excludes our own
        // overlay windows — the canvas, pucks, and box all live at floating /
        // status levels — so annotating the Bang window itself works (its main
        // window is layer 0), while the transparent canvas on top is ignored.
        if (layer != 0) {
            continue; // skip menu bar, dock, floating overlays, etc.
        }
        CFNumberRef pidRef = (CFNumberRef)CFDictionaryGetValue(info, kCGWindowOwnerPID);
        int pid = 0;
        if (pidRef != NULL) {
            CFNumberGetValue(pidRef, kCFNumberIntType, &pid);
        }
        CFDictionaryRef boundsDict = (CFDictionaryRef)CFDictionaryGetValue(info, kCGWindowBounds);
        if (boundsDict == NULL) {
            continue;
        }
        CGRect rect;
        if (!CGRectMakeWithDictionaryRepresentation(boundsDict, &rect)) {
            continue;
        }
        if (CGRectContainsPoint(rect, p)) {
            *out = rect;
            if (outPid != NULL) {
                *outPid = (pid_t)pid;
            }
            found = YES;
            break; // front-to-back order: first hit is frontmost.
        }
    }
    CFRelease(list);
    return found;
}

// A borderless panel that can still become key so it reliably receives clicks +
// keyboard (Return/Esc) without activating our app over the target window.
@interface BangCanvasPanel : NSPanel
@end
@implementation BangCanvasPanel
- (BOOL)canBecomeKeyWindow {
    return YES;
}
@end

@interface BangCanvasView : NSView {
    NSMutableArray<NSBezierPath *> *_strokes;
    NSBezierPath *_current;
    BOOL _hasPick;
    CGRect _pickCg;   // picked window, CG global (top-left), points
    NSRect _pickView; // picked window in this view's coords
    BOOL _capturing;  // Done pressed: draw strokes only (no tint/outline/chrome)
    NSView *_controls; // opaque toolbar holding Done/Cancel
    NSButton *_doneButton;
    NSButton *_cancelButton;
}
@end

@implementation BangCanvasView
- (instancetype)initWithFrame:(NSRect)frame {
    self = [super initWithFrame:frame];
    if (self != nil) {
        _strokes = [NSMutableArray array];
        self.wantsLayer = YES;
    }
    return self;
}
// Draw on the very first click even when our app/window isn't active.
- (BOOL)acceptsFirstMouse:(NSEvent *)event {
    (void)event;
    return YES;
}
// Show a crosshair over the whole canvas so it's clear you can draw.
- (void)resetCursorRects {
    [self addCursorRect:self.bounds cursor:[NSCursor crosshairCursor]];
}
- (void)setControls:(NSView *)controls done:(NSButton *)done cancel:(NSButton *)cancel {
    _controls = controls;
    _doneButton = done;
    _cancelButton = cancel;
}
- (void)drawRect:(NSRect)dirtyRect {
    (void)dirtyRect;
    if (!_capturing) {
        // Faint tint marks the annotation surface (removed before capture).
        [[NSColor colorWithSRGBRed:0.0 green:0.0 blue:0.0 alpha:0.12] setFill];
        NSRectFillUsingOperation(self.bounds, NSCompositingOperationSourceOver);
    }
    if (!_capturing && _hasPick) {
        [BANG_INK setStroke];
        NSBezierPath *outline = [NSBezierPath bezierPathWithRect:NSInsetRect(_pickView, 1.0, 1.0)];
        outline.lineWidth = 2.0;
        [outline stroke];
    }
    [BANG_INK setStroke];
    for (NSBezierPath *p in _strokes) {
        p.lineWidth = 4.0;
        p.lineCapStyle = NSLineCapStyleRound;
        p.lineJoinStyle = NSLineJoinStyleRound;
        [p stroke];
    }
    if (_current != nil) {
        _current.lineWidth = 4.0;
        _current.lineCapStyle = NSLineCapStyleRound;
        _current.lineJoinStyle = NSLineJoinStyleRound;
        [_current stroke];
    }
}
// Move the controls toolbar to the top of the currently picked window so it
// follows the selection.
- (void)repositionButtons {
    if (!_hasPick || _controls == nil) {
        return;
    }
    NSSize c = _controls.frame.size;
    NSRect vb = self.bounds;
    CGFloat x = NSMidX(_pickView) - c.width / 2.0;
    CGFloat y = NSMaxY(_pickView) - c.height - 10.0; // just inside the window's top
    x = MIN(MAX(x, NSMinX(vb) + 4.0), NSMaxX(vb) - c.width - 4.0);
    y = MIN(MAX(y, NSMinY(vb) + 4.0), NSMaxY(vb) - c.height - 4.0);
    _controls.autoresizingMask = NSViewNotSizable;
    [_controls setFrameOrigin:NSMakePoint(x, y)];
    // Cursor rects are cached in window coords; refresh them now that the
    // buttons moved so the finger cursor tracks their new position.
    for (NSView *button in _controls.subviews) {
        [self.window invalidateCursorRectsForView:button];
    }
}
- (void)mouseDown:(NSEvent *)event {
    NSPoint p = [self convertPoint:event.locationInWindow fromView:nil];
    // Re-pick on every stroke start: the window you draw on becomes the target,
    // so drawing on a different window re-selects it (and moves the controls).
    CGEventRef ev = CGEventCreate(NULL);
    if (ev != NULL) {
        CGPoint cg = CGEventGetLocation(ev);
        CFRelease(ev);
        CGRect rect;
        pid_t pid = 0;
        if (bangWindowUnderPoint(cg, &rect, &pid)) {
            if (!_hasPick || !CGRectEqualToRect(rect, _pickCg)) {
                // Switched to a different window: start a fresh annotation and
                // raise that window's app to the front so it's clearly focused.
                [_strokes removeAllObjects];
                NSRunningApplication *target =
                    [NSRunningApplication runningApplicationWithProcessIdentifier:pid];
                [target activateWithOptions:NSApplicationActivateAllWindows];
            }
            _hasPick = YES;
            _pickCg = rect;
            // Calibrate CG(top-left) -> view(bottom-left) using this very click,
            // which we know in both coordinate spaces — exact on any display.
            CGFloat offsetX = p.x - cg.x;
            CGFloat flipBase = p.y + cg.y;
            _pickView = NSMakeRect(rect.origin.x + offsetX,
                                   flipBase - (rect.origin.y + rect.size.height),
                                   rect.size.width, rect.size.height);
            [self repositionButtons];
        }
    }
    // Reclaim key focus (raising the picked window's app took it) so Cmd+Z lands
    // on the canvas while you're drawing.
    [self.window makeKeyWindow];
    _current = [NSBezierPath bezierPath];
    [_current moveToPoint:p];
    [self setNeedsDisplay:YES];
}
- (void)mouseDragged:(NSEvent *)event {
    if (_current == nil) {
        return;
    }
    [_current lineToPoint:[self convertPoint:event.locationInWindow fromView:nil]];
    [self setNeedsDisplay:YES];
}
- (void)mouseUp:(NSEvent *)event {
    (void)event;
    if (_current != nil) {
        [_strokes addObject:_current];
        _current = nil;
        [self setNeedsDisplay:YES];
    }
}
- (void)done:(id)sender {
    (void)sender;
    if (!_hasPick) {
        bang_overlay_canvas_cancel(); // nothing drawn -> treat as cancel
        return;
    }
    // Hide chrome so only the window + strokes are captured.
    _capturing = YES;
    _controls.hidden = YES;
    [self setNeedsDisplay:YES];
    [self displayIfNeeded];
    CGRect r = _pickCg;
    // Defer so the compositor presents the chrome-free frame before capture.
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.05 * NSEC_PER_SEC)),
                   dispatch_get_main_queue(), ^{
                     bang_overlay_canvas_done(r.origin.x, r.origin.y, r.size.width, r.size.height);
                   });
}
- (void)cancel:(id)sender {
    (void)sender;
    bang_overlay_canvas_cancel();
}
// Undo the last drawn stroke (Undo button / Cmd+Z). Discards an in-progress
// stroke first, then pops the most recent completed one.
- (void)undo:(id)sender {
    (void)sender;
    if (_current != nil) {
        _current = nil;
    } else if (_strokes.count > 0) {
        [_strokes removeLastObject];
    }
    [self setNeedsDisplay:YES];
}
@end

static NSPanel *gCanvasPanel = nil;
static BangCanvasView *gCanvasView = nil;

// A button that shows the pointing-hand ("finger") cursor on hover.
@interface BangCursorButton : NSButton
@end
@implementation BangCursorButton
- (void)resetCursorRects {
    [self addCursorRect:self.bounds cursor:[NSCursor pointingHandCursor]];
}
@end

static NSButton *bangCanvasButton(NSString *title, NSRect frame, id target, SEL action,
                                  NSString *keyEquivalent) {
    NSButton *button = [[BangCursorButton alloc] initWithFrame:frame];
    button.title = title;
    button.bezelStyle = NSBezelStyleRounded;
    [button setButtonType:NSButtonTypeMomentaryPushIn];
    button.target = target;
    button.action = action;
    button.keyEquivalent = keyEquivalent;
    button.autoresizingMask = NSViewMinXMargin | NSViewMaxXMargin | NSViewMinYMargin;
    return button;
}

// Recreated on each show so previous strokes / picked window don't linger.
void bang_overlay_canvas_show(void) {
    if (gCanvasPanel != nil) {
        [gCanvasPanel orderOut:nil];
        gCanvasPanel = nil;
        gCanvasView = nil;
    }
    NSScreen *screen = [NSScreen mainScreen];
    NSRect frame = (screen != nil) ? screen.frame : NSMakeRect(0, 0, 1440, 900);

    BangCanvasPanel *panel = [[BangCanvasPanel alloc]
        initWithContentRect:frame
                  styleMask:(NSWindowStyleMaskBorderless | NSWindowStyleMaskNonactivatingPanel)
                    backing:NSBackingStoreBuffered
                      defer:NO];
    panel.level = NSStatusWindowLevel; // above the floating pucks
    panel.opaque = NO;
    panel.backgroundColor = [NSColor clearColor];
    panel.hasShadow = NO;
    panel.collectionBehavior =
        NSWindowCollectionBehaviorCanJoinAllSpaces | NSWindowCollectionBehaviorFullScreenAuxiliary;
    panel.releasedWhenClosed = NO;

    BangCanvasView *view =
        [[BangCanvasView alloc] initWithFrame:NSMakeRect(0, 0, frame.size.width, frame.size.height)];
    view.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;

    // Opaque toolbar holding Undo + Cancel + Done (so the controls read as solid,
    // not see-through over the transparent canvas).
    const CGFloat bw = 84.0, bh = 28.0, pad = 8.0, gap = 10.0;
    CGFloat cw = bw * 3.0 + gap * 2.0 + pad * 2.0;
    CGFloat ch = bh + pad * 2.0;
    NSView *controls = [[NSView alloc]
        initWithFrame:NSMakeRect((frame.size.width - cw) / 2.0, frame.size.height - 90.0, cw, ch)];
    controls.wantsLayer = YES;
    controls.layer.backgroundColor = [[NSColor colorWithWhite:0.14 alpha:1.0] CGColor];
    controls.layer.cornerRadius = 9.0;
    NSButton *undo = bangCanvasButton(@"Undo", NSMakeRect(pad, pad, bw, bh), view,
                                      @selector(undo:), @"z");
    undo.keyEquivalentModifierMask = NSEventModifierFlagCommand; // Cmd+Z
    NSButton *cancel = bangCanvasButton(@"Cancel", NSMakeRect(pad + bw + gap, pad, bw, bh), view,
                                        @selector(cancel:), @"\033");
    NSButton *done = bangCanvasButton(@"Done", NSMakeRect(pad + (bw + gap) * 2.0, pad, bw, bh), view,
                                      @selector(done:), @"\r");
    [controls addSubview:undo];
    [controls addSubview:cancel];
    [controls addSubview:done];
    [view addSubview:controls];
    [view setControls:controls done:done cancel:cancel];

    panel.contentView = view;
    gCanvasPanel = panel;
    gCanvasView = view;
    [gCanvasPanel orderFrontRegardless];
    // Become key (without activating the app) so clicks + Return/Esc land here.
    [gCanvasPanel makeKeyWindow];
    [gCanvasPanel invalidateCursorRectsForView:view];
}

void bang_overlay_canvas_hide(void) {
    if (gCanvasPanel != nil) {
        [gCanvasPanel orderOut:nil];
        gCanvasPanel = nil;
        gCanvasView = nil;
    }
}
