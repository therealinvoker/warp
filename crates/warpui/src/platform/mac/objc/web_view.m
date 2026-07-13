#import <AppKit/AppKit.h>
#import <WebKit/WebKit.h>
#import <objc/runtime.h>

#import "web_view.h"

// Per-window map of web view id -> WKWebView, stored as an associated object on
// the NSWindow so its lifetime is tied to the window. Uses manual reference
// counting (this file, like the rest of the objc sources, is compiled without
// ARC).
static const void *kWarpWebViewsAssocKey = &kWarpWebViewsAssocKey;

static NSMutableDictionary<NSString *, WKWebView *> *warpWebViewsForWindow(NSWindow *window) {
    NSMutableDictionary<NSString *, WKWebView *> *views =
        objc_getAssociatedObject(window, kWarpWebViewsAssocKey);
    if (views == nil) {
        views = [NSMutableDictionary dictionary];
        objc_setAssociatedObject(window, kWarpWebViewsAssocKey, views,
                                 OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    }
    return views;
}

void warp_web_view_ensure(NSWindow *window, NSString *viewId) {
    if (window == nil || viewId == nil) {
        return;
    }
    NSView *contentView = window.contentView;
    if (contentView == nil) {
        return;
    }
    NSMutableDictionary<NSString *, WKWebView *> *views = warpWebViewsForWindow(window);
    if (views[viewId] != nil) {
        return;
    }

    WKWebViewConfiguration *config = [[[WKWebViewConfiguration alloc] init] autorelease];
    WKWebView *webView = [[[WKWebView alloc] initWithFrame:NSMakeRect(0, 0, 0, 0)
                                             configuration:config] autorelease];
    // We drive the frame explicitly every frame from the WarpUI layout rect, so
    // disable AppKit autoresizing.
    webView.autoresizingMask = NSViewNotSizable;
    // Start hidden; the caller reveals it once positioned.
    webView.hidden = YES;
    [contentView addSubview:webView];
    views[viewId] = webView;
}

void warp_web_view_navigate(NSWindow *window, NSString *viewId, NSString *urlString) {
    if (window == nil || viewId == nil || urlString == nil) {
        return;
    }
    WKWebView *webView = warpWebViewsForWindow(window)[viewId];
    if (webView == nil) {
        return;
    }
    NSURL *url = [NSURL URLWithString:urlString];
    if (url == nil) {
        return;
    }
    [webView loadRequest:[NSURLRequest requestWithURL:url]];
}

void warp_web_view_set_frame(NSWindow *window, NSString *viewId, NSRect frame) {
    if (window == nil || viewId == nil) {
        return;
    }
    WKWebView *webView = warpWebViewsForWindow(window)[viewId];
    if (webView == nil) {
        return;
    }
    [webView setFrame:frame];
}

void warp_web_view_set_hidden(NSWindow *window, NSString *viewId, bool hidden) {
    if (window == nil || viewId == nil) {
        return;
    }
    WKWebView *webView = warpWebViewsForWindow(window)[viewId];
    if (webView == nil) {
        return;
    }
    webView.hidden = hidden ? YES : NO;
}

void warp_web_view_reload(NSWindow *window, NSString *viewId) {
    if (window == nil || viewId == nil) {
        return;
    }
    WKWebView *webView = warpWebViewsForWindow(window)[viewId];
    [webView reload];
}

void warp_web_view_go_back(NSWindow *window, NSString *viewId) {
    if (window == nil || viewId == nil) {
        return;
    }
    WKWebView *webView = warpWebViewsForWindow(window)[viewId];
    [webView goBack];
}

void warp_web_view_go_forward(NSWindow *window, NSString *viewId) {
    if (window == nil || viewId == nil) {
        return;
    }
    WKWebView *webView = warpWebViewsForWindow(window)[viewId];
    [webView goForward];
}

void warp_web_view_destroy(NSWindow *window, NSString *viewId) {
    if (window == nil || viewId == nil) {
        return;
    }
    NSMutableDictionary<NSString *, WKWebView *> *views = warpWebViewsForWindow(window);
    WKWebView *webView = views[viewId];
    if (webView == nil) {
        return;
    }
    [webView removeFromSuperview];
    [views removeObjectForKey:viewId];
}
