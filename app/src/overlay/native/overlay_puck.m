// Native macOS "puck" windows for the Bang voice + annotation overlay.
//
// Two borderless, always-on-top, transparent, circular panels:
//   - the MIC puck (kind 0): gradient + lightning bolt; reflects listening /
//     paused state and modulates its glow with the live mic level. Clicking it
//     pauses/resumes recording.
//   - the SUBMIT puck (kind 1): an up-arrow; clicking it submits the transcript.
//
// Controlled from Rust (app/src/overlay/platform_mac.rs) via the
// `bang_overlay_puck_*` C functions. Clicks are routed back into the app via the
// Rust entrypoint `bang_overlay_puck_clicked(kind)`. Compiled with ARC (see
// app/build.rs). All functions run on the main thread (AppKit requirement).

#import <Cocoa/Cocoa.h>
#import <stdbool.h>

// Implemented in Rust (app/src/overlay/platform_mac.rs).
extern void bang_overlay_puck_clicked(int kind);
extern void bang_overlay_box_edited(const char *utf8);
extern void bang_overlay_auto_submit_clicked(void);

// Header strip at the top of the result box for its controls (auto-submit
// toggle on the left, expand/collapse on the right).
static const CGFloat kBoxHeaderH = 22.0;

enum { BangPuckKindMic = 0, BangPuckKindSubmit = 1 };

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
@property(nonatomic, assign) NSPoint dragStartMouse;
@property(nonatomic, assign) NSPoint dragStartOrigin;
@property(nonatomic, assign) BOOL didDrag;
@end

@implementation BangPuckView

- (void)drawGradientCircleInRect:(NSRect)rect dim:(BOOL)dim {
    NSBezierPath *circle = [NSBezierPath bezierPathWithOvalInRect:rect];
    NSColor *startColor;
    NSColor *endColor;
    if (dim) {
        startColor = [NSColor colorWithSRGBRed:0.32 green:0.32 blue:0.36 alpha:1.0];
        endColor = [NSColor colorWithSRGBRed:0.20 green:0.20 blue:0.24 alpha:1.0];
    } else {
        startColor = [NSColor colorWithSRGBRed:1.0 green:0.176 blue:0.471 alpha:1.0]; // #FF2D78
        endColor = [NSColor colorWithSRGBRed:0.302 green:0.486 blue:1.0 alpha:1.0];   // #4D7CFF
    }
    NSGradient *gradient = [[NSGradient alloc] initWithStartingColor:startColor endingColor:endColor];
    // Rotate the gradient with the live level so it visibly "modulates" while
    // the user is talking.
    CGFloat angle = 135.0 + (dim ? 0.0 : self.level * 240.0);
    [gradient drawInBezierPath:circle angle:angle];
}

- (void)fillGlyphPath:(NSBezierPath *)path white:(BOOL)white {
    [(white ? [NSColor whiteColor] : [NSColor colorWithWhite:0.85 alpha:1.0]) setFill];
    [path fill];
}

- (void)drawBoltInRect:(NSRect)bounds {
    NSRect inner = NSInsetRect(bounds, 3.0, 3.0);
    CGFloat side = MIN(inner.size.width, inner.size.height) * 0.5;
    CGFloat ox = NSMidX(bounds) - side / 2.0;
    CGFloat oy = NSMidY(bounds) - side / 2.0;
    const CGFloat pts[][2] = {
        {0.52, 0.98}, {0.20, 0.50}, {0.44, 0.50},
        {0.34, 0.02}, {0.80, 0.56}, {0.54, 0.56},
    };
    NSBezierPath *bolt = [NSBezierPath bezierPath];
    for (int i = 0; i < 6; i++) {
        NSPoint p = NSMakePoint(ox + pts[i][0] * side, oy + pts[i][1] * side);
        if (i == 0) {
            [bolt moveToPoint:p];
        } else {
            [bolt lineToPoint:p];
        }
    }
    [bolt closePath];
    [self fillGlyphPath:bolt white:!self.paused];
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
    [[NSColor whiteColor] setStroke];
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
    NSRect circleRect = NSInsetRect(bounds, 3.0, 3.0);

    if (self.kind == BangPuckKindSubmit) {
        [self drawGradientCircleInRect:circleRect dim:NO];
        if (self.thinking) {
            [self drawSpotlightInRect:bounds];
        }
        [self drawUpArrowInRect:bounds];
        return;
    }

    // Mic puck.
    // Outer glow ring that grows/brightens with the live mic level — the
    // "modulation while talking" effect. Suppressed while paused.
    if (!self.paused && self.level > 0.001) {
        CGFloat mod = self.level * 6.0;
        if (mod > 1.0) {
            mod = 1.0;
        }
        CGFloat grow = mod * 3.0;
        NSRect glowRect = NSInsetRect(bounds, 2.0 - grow, 2.0 - grow);
        NSBezierPath *glow = [NSBezierPath bezierPathWithOvalInRect:glowRect];
        glow.lineWidth = 1.5 + mod * 2.5;
        [[NSColor colorWithSRGBRed:1.0 green:0.4 blue:0.6 alpha:0.25 + mod * 0.5] setStroke];
        [glow stroke];
    }

    [self drawGradientCircleInRect:circleRect dim:self.paused];
    [self drawBoltInRect:bounds];

    if (self.listening && !self.paused) {
        NSBezierPath *ring = [NSBezierPath bezierPathWithOvalInRect:NSInsetRect(bounds, 1.0, 1.0)];
        ring.lineWidth = 2.0;
        [[NSColor whiteColor] setStroke];
        [ring stroke];
    }
}

// Manual dragging + click detection. `movableByWindowBackground` would swallow
// clicks, so we move the window ourselves and treat a mouse-up without drag as a
// click.
- (void)mouseDown:(NSEvent *)event {
    (void)event;
    self.didDrag = NO;
    self.dragStartMouse = [NSEvent mouseLocation];
    self.dragStartOrigin = self.window.frame.origin;
}

- (void)mouseDragged:(NSEvent *)event {
    (void)event;
    self.didDrag = YES;
    NSPoint now = [NSEvent mouseLocation];
    NSPoint origin = self.dragStartOrigin;
    origin.x += now.x - self.dragStartMouse.x;
    origin.y += now.y - self.dragStartMouse.y;
    [self.window setFrameOrigin:origin];
}

- (void)mouseUp:(NSEvent *)event {
    (void)event;
    if (!self.didDrag) {
        bang_overlay_puck_clicked(self.kind);
    }
}

@end

static NSPanel *gMicPuck = nil;
static NSPanel *gSubmitPuck = nil;

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

static void bangEnsurePucks(void) {
    if (gMicPuck != nil) {
        return;
    }
    NSScreen *screen = [NSScreen mainScreen];
    NSRect visible = (screen != nil) ? [screen visibleFrame] : NSMakeRect(0, 0, 1440, 900);
    const CGFloat size = 64.0;
    const CGFloat margin = 24.0;
    const CGFloat gap = 12.0;

    NSRect micFrame =
        NSMakeRect(NSMaxX(visible) - size - margin, NSMinY(visible) + margin, size, size);
    // Submit puck sits just to the left of the mic puck.
    NSRect submitFrame = NSMakeRect(micFrame.origin.x - size - gap, micFrame.origin.y, size, size);

    gMicPuck = bangMakePuck(BangPuckKindMic, micFrame);
    gSubmitPuck = bangMakePuck(BangPuckKindSubmit, submitFrame);
}

static BangPuckView *bangMicView(void) {
    return gMicPuck != nil ? (BangPuckView *)gMicPuck.contentView : nil;
}

static BangPuckView *bangSubmitView(void) {
    return gSubmitPuck != nil ? (BangPuckView *)gSubmitPuck.contentView : nil;
}

void bang_overlay_puck_show(void) {
    bangEnsurePucks();
    [gSubmitPuck orderFrontRegardless];
    [gMicPuck orderFrontRegardless];
}

void bang_overlay_puck_hide(void) {
    if (gMicPuck != nil) {
        [gMicPuck orderOut:nil];
    }
    if (gSubmitPuck != nil) {
        [gSubmitPuck orderOut:nil];
    }
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
    BangPuckView *view = bangSubmitView();
    if (view != nil) {
        view.thinking = thinking ? YES : NO;
        if (thinking) {
            [view startSpin];
        } else {
            [view stopSpin];
        }
        [view setNeedsDisplay:YES];
    }
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
// Set while we programmatically update the text view (see
// bang_overlay_box_set_text) so the resulting textDidChange isn't mistaken for a
// user edit — which would re-enter the Rust app context and crash.
static BOOL gBoxProgrammaticSet = NO;
// Box height cap: 40 lines when expanded, 4 when collapsed. The box auto-grows
// with its content up to the cap, then scrolls.
static BOOL gBoxExpanded = YES;
static NSButton *gBoxExpander = nil;
static NSButton *gBoxAutoToggle = nil;

enum { BangBoxMaxLines = 40, BangBoxMinLines = 4 };

static void bangLayoutBox(void);

static NSString *bangExpanderTitle(void) {
    // ⌃ collapse (currently expanded) / ⌄ expand (currently collapsed).
    return gBoxExpanded ? @"\u2303" : @"\u2304";
}

static void bangSetExpanderTitle(NSButton *button) {
    NSDictionary *attrs = @{
        NSForegroundColorAttributeName : [NSColor whiteColor],
        NSFontAttributeName : [NSFont systemFontOfSize:12.0]
    };
    button.attributedTitle = [[NSAttributedString alloc] initWithString:bangExpanderTitle()
                                                             attributes:attrs];
}

/// Forwards user edits in the box text view back to the composer, and toggles
/// the box height cap (expander button).
@interface BangBoxDelegate : NSObject <NSTextViewDelegate>
@end
@implementation BangBoxDelegate
- (void)textDidBeginEditing:(NSNotification *)notification {
    (void)notification;
    gBoxEditing = YES;
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
- (void)toggleExpand:(id)sender {
    gBoxExpanded = !gBoxExpanded;
    if ([sender isKindOfClass:[NSButton class]]) {
        bangSetExpanderTitle((NSButton *)sender);
    }
    bangLayoutBox();
}
- (void)toggleAutoSubmit:(id)sender {
    (void)sender;
    // Rust flips the persisted setting and pushes the new state back via
    // bang_overlay_box_set_auto_submit.
    bang_overlay_auto_submit_clicked();
}
@end

static NSPanel *gBox = nil;
static NSTextView *gBoxTextView = nil;
static BangBoxDelegate *gBoxDelegate = nil;

static void bangEnsureBox(void) {
    if (gBox != nil) {
        return;
    }
    const CGFloat width = 400.0;
    const CGFloat height = 64.0;
    const CGFloat gap = 12.0;

    NSRect frame;
    if (gMicPuck != nil) {
        NSRect mic = gMicPuck.frame;
        // Right-aligned with the mic puck, sitting just above the puck row.
        frame = NSMakeRect(NSMaxX(mic) - width, NSMaxY(mic) + gap, width, height);
    } else {
        NSScreen *screen = [NSScreen mainScreen];
        NSRect visible = (screen != nil) ? [screen visibleFrame] : NSMakeRect(0, 0, 1440, 900);
        frame = NSMakeRect(NSMaxX(visible) - width - 24.0, NSMinY(visible) + 100.0, width, height);
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
    bg.layer.backgroundColor = [[NSColor colorWithWhite:0.0 alpha:0.6] CGColor];
    bg.layer.cornerRadius = 10.0;
    bg.layer.masksToBounds = YES;

    // Text scroll fills below the header strip.
    NSRect scrollFrame = NSMakeRect(8.0, 6.0, width - 16.0, height - kBoxHeaderH - 6.0);
    NSScrollView *scroll = [[NSScrollView alloc] initWithFrame:scrollFrame];
    scroll.drawsBackground = NO;
    scroll.hasVerticalScroller = YES;
    scroll.autohidesScrollers = YES;
    scroll.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;

    NSTextView *tv = [[NSTextView alloc] initWithFrame:scroll.bounds];
    tv.editable = YES; // dictation mode by default; toggled per mode
    tv.selectable = YES;
    tv.drawsBackground = NO;
    tv.textColor = [NSColor whiteColor];
    tv.insertionPointColor = [NSColor whiteColor];
    tv.font = [NSFont systemFontOfSize:11.0];
    tv.textContainerInset = NSMakeSize(2.0, 2.0);
    tv.verticallyResizable = YES;
    tv.horizontallyResizable = NO;
    [tv.textContainer setWidthTracksTextView:YES];
    gBoxDelegate = [[BangBoxDelegate alloc] init];
    tv.delegate = gBoxDelegate;
    scroll.documentView = tv;

    [bg addSubview:scroll];

    CGFloat headerY = height - kBoxHeaderH + 3.0;

    // Auto-submit toggle (hands-free vs. manual submit). Pinned top-left.
    NSButton *autoToggle =
        [[NSButton alloc] initWithFrame:NSMakeRect(8.0, headerY, 160.0, 16.0)];
    [autoToggle setButtonType:NSButtonTypeSwitch];
    autoToggle.target = gBoxDelegate;
    autoToggle.action = @selector(toggleAutoSubmit:);
    autoToggle.autoresizingMask = NSViewMaxXMargin | NSViewMinYMargin;
    autoToggle.attributedTitle = [[NSAttributedString alloc]
        initWithString:@"Auto-submit"
            attributes:@{
                NSForegroundColorAttributeName : [NSColor whiteColor],
                NSFontAttributeName : [NSFont systemFontOfSize:11.0]
            }];
    [bg addSubview:autoToggle];
    gBoxAutoToggle = autoToggle;

    // Expander: toggles the height cap between 40 and 4 lines. Pinned top-right.
    NSButton *expander =
        [[NSButton alloc] initWithFrame:NSMakeRect(width - 24.0, headerY, 18.0, 16.0)];
    expander.bordered = NO;
    [expander setButtonType:NSButtonTypeMomentaryChange];
    expander.target = gBoxDelegate;
    expander.action = @selector(toggleExpand:);
    expander.autoresizingMask = NSViewMinXMargin | NSViewMinYMargin;
    bangSetExpanderTitle(expander);
    [bg addSubview:expander];
    gBoxExpander = expander;

    panel.contentView = bg;
    gBox = panel;
    gBoxTextView = tv;
    bangLayoutBox();
}

// Resize the box to fit its content, capped at 40 (expanded) or 4 (collapsed)
// lines. Grows/shrinks upward so it never overlaps the pucks below it.
static void bangLayoutBox(void) {
    if (gBox == nil || gBoxTextView == nil) {
        return;
    }
    NSFont *font = gBoxTextView.font ?: [NSFont systemFontOfSize:11.0];
    NSLayoutManager *probe = [[NSLayoutManager alloc] init];
    CGFloat lineH = [probe defaultLineHeightForFont:font];
    if (lineH <= 0.0) {
        lineH = 14.0;
    }
    NSInteger cap = gBoxExpanded ? BangBoxMaxLines : BangBoxMinLines;
    CGFloat maxInner = cap * lineH;
    CGFloat minInner = lineH;

    [gBoxTextView.layoutManager ensureLayoutForTextContainer:gBoxTextView.textContainer];
    NSRect used = [gBoxTextView.layoutManager usedRectForTextContainer:gBoxTextView.textContainer];
    CGFloat innerH = used.size.height;
    if (innerH < minInner) {
        innerH = minInner;
    }
    if (innerH > maxInner) {
        innerH = maxInner;
    }

    // scroll (12) + text container (4) vertical insets + the control header.
    const CGFloat chrome = 16.0 + kBoxHeaderH;
    CGFloat panelH = ceil(innerH + chrome);

    NSRect f = gBox.frame;
    if (fabs(f.size.height - panelH) < 0.5) {
        return;
    }
    // Keep origin.y (the bottom edge) fixed so the box grows/shrinks upward.
    f.size.height = panelH;
    [gBox setFrame:f display:YES];
    [gBoxTextView scrollRangeToVisible:NSMakeRange((NSInteger)gBoxTextView.string.length, 0)];
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

// Raw source of the last text set programmatically, so streaming updates can be
// deduped even though result mode stores an attributed (markdown-rendered) copy
// whose `.string` differs from the source.
static NSString *gBoxLastRaw = nil;

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
    return result;
}

void bang_overlay_box_set_text(const char *utf8) {
    if (gBoxTextView == nil || utf8 == NULL) {
        return;
    }
    // Don't clobber what the user is actively typing in the box.
    if (gBoxEditing) {
        return;
    }
    NSString *text = [NSString stringWithUTF8String:utf8];
    if (text == nil) {
        return;
    }
    if (gBoxLastRaw != nil && [text isEqualToString:gBoxLastRaw]) {
        return;
    }
    gBoxLastRaw = [text copy];
    gBoxProgrammaticSet = YES;
    if (gBoxTextView.editable) {
        // Dictation mode: plain, editable transcript.
        gBoxTextView.string = text;
    } else {
        // Result mode: render markdown so it matches the main UI's formatting.
        [gBoxTextView.textStorage setAttributedString:bangRenderMarkdown(text)];
    }
    gBoxProgrammaticSet = NO;
    bangLayoutBox();
    [gBoxTextView scrollRangeToVisible:NSMakeRange((NSInteger)gBoxTextView.string.length, 0)];
}

void bang_overlay_box_set_editable(bool editable) {
    if (gBoxTextView != nil) {
        gBoxTextView.editable = editable ? YES : NO;
        // Force the next set_text to re-render for the new mode (plain vs markdown).
        gBoxLastRaw = nil;
    }
}

// Reflect the persisted auto-submit setting on the box's toggle.
void bang_overlay_box_set_auto_submit(bool on) {
    if (gBoxAutoToggle != nil) {
        gBoxAutoToggle.state = on ? NSControlStateValueOn : NSControlStateValueOff;
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
        bg.layer.backgroundColor =
            [[NSColor colorWithSRGBRed:r green:g blue:b alpha:a] CGColor];
    }
}
