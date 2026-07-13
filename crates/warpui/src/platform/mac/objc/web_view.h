#import <AppKit/AppKit.h>

// Native embedded web view support for the Bang "Preview" tools tab.
//
// Each window can host any number of WKWebViews, keyed by a string id. The web
// views are added as subviews of the window's content view (the layer-backed
// WarpHostView), so AppKit composites them above the Metal-rendered UI. The
// caller is responsible for driving frame/visibility each frame based on the
// laid-out WarpUI element rect.
//
// Frames passed to `warp_web_view_set_frame` are in AppKit content-view
// coordinates (bottom-left origin, points); the Rust caller performs the
// coordinate conversion.

void warp_web_view_ensure(NSWindow *window, NSString *viewId);
void warp_web_view_navigate(NSWindow *window, NSString *viewId, NSString *urlString);
void warp_web_view_set_frame(NSWindow *window, NSString *viewId, NSRect frame);
void warp_web_view_set_hidden(NSWindow *window, NSString *viewId, bool hidden);
void warp_web_view_reload(NSWindow *window, NSString *viewId);
void warp_web_view_go_back(NSWindow *window, NSString *viewId);
void warp_web_view_go_forward(NSWindow *window, NSString *viewId);
void warp_web_view_destroy(NSWindow *window, NSString *viewId);
