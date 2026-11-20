// macOS: every way of quitting asks the close guard first.
//
// The app menu's Quit, Cmd+Q, Quit in the Dock, an Apple Event quit
// ("tell application ... to quit") and logging out all reach -[NSApplication
// terminate:]. The windowing layer implements only applicationWillTerminate:,
// which runs when the process is already ending, so none of these used to ask
// whether a recording or an export was running. This adds
// applicationShouldTerminate: to the application delegate. The callback holds
// the quit (NSTerminateCancel) while the guard saves the recording or asks
// about the export; the guard then quits the app itself.

#import <AppKit/AppKit.h>
#import <objc/message.h>
#import <objc/runtime.h>
#include <stdio.h>

typedef int (*sxc_quit_callback)(void);

static sxc_quit_callback g_callback = NULL;
static IMP g_original = NULL;

static NSApplicationTerminateReply sxc_should_terminate(id self, SEL cmd, NSApplication *sender) {
    if (g_callback != NULL && g_callback() != 0) {
        return NSTerminateCancel;
    }
    if (g_original != NULL) {
        return ((NSApplicationTerminateReply (*)(id, SEL, NSApplication *))g_original)(self, cmd, sender);
    }
    return NSTerminateNow;
}

static int install_on_class(Class cls, sxc_quit_callback callback) {
    g_callback = callback;
    SEL sel = @selector(applicationShouldTerminate:);
    Method existing = class_getInstanceMethod(cls, sel);
    if (existing != NULL) {
        // A later windowing layer may answer this itself: ask the guard first,
        // then let it answer.
        IMP previous = method_getImplementation(existing);
        if (previous != (IMP)sxc_should_terminate) {
            g_original = method_setImplementation(existing, (IMP)sxc_should_terminate);
        }
        return 0;
    }
    g_original = NULL;
    // NSApplicationTerminateReply is an NSUInteger: "Q" on 64-bit macOS.
    return class_addMethod(cls, sel, (IMP)sxc_should_terminate, "Q@:@") ? 0 : 3;
}

// 0 installed; 1 no application delegate; 3 the method could not be added;
// 4 not called on the main thread.
int sxc_macos_install_quit_hook(sxc_quit_callback callback) {
    if (![NSThread isMainThread]) {
        return 4;
    }
    id delegate = [NSApp delegate];
    if (delegate == nil) {
        return 1;
    }
    return install_on_class(object_getClass(delegate), callback);
}

// Test support: send applicationShouldTerminate: (what -[NSApplication
// terminate:] sends first) to a fresh delegate class with the hook installed,
// with or without its own answer, and return the reply. No application object
// is involved and nothing terminates.
static int g_selftest_hold = 0;
static int selftest_callback(void) { return g_selftest_hold; }
static NSApplicationTerminateReply selftest_own_answer(id self, SEL cmd, NSApplication *sender) {
    (void)self;
    (void)cmd;
    (void)sender;
    return NSTerminateLater;
}

long sxc_macos_quit_hook_selftest(int hold, int delegate_answers_itself) {
    static int serial = 0;
    char name[64];
    snprintf(name, sizeof name, "SxcQuitHookSelfTest%d", ++serial);
    Class cls = objc_allocateClassPair([NSObject class], name, 0);
    if (cls == Nil) {
        return -1;
    }
    if (delegate_answers_itself) {
        class_addMethod(cls, @selector(applicationShouldTerminate:), (IMP)selftest_own_answer, "Q@:@");
    }
    objc_registerClassPair(cls);
    g_selftest_hold = hold;
    if (install_on_class(cls, selftest_callback) != 0) {
        return -2;
    }
    id delegate = [[cls alloc] init];
    NSApplicationTerminateReply (*send)(id, SEL, id) =
        (NSApplicationTerminateReply (*)(id, SEL, id))objc_msgSend;
    return (long)send(delegate, @selector(applicationShouldTerminate:), nil);
}
