// macos_region_picker.mm — private visual macOS display-region picker.
//
// This is deliberately an AppKit-only, synchronous modal overlay. It accepts
// no coordinates and returns no image, title, display handle, or screen-space
// geometry to Rust: after explicit Enter confirmation it emits only a
// display-local, even native-pixel crop plus the transient display number. It
// is compiled into the foreground Tauri shell (not cutd); the authenticated
// child-only handoff immediately replaces that number with cutd's opaque exact
// monitor identity and burns a server-private one-use ticket. Escape is a
// cancellation; an invalid/changed visual selection refuses in place.

#import <AppKit/AppKit.h>
#include <algorithm>
#include <cmath>
#include <cstdint>
#include <limits>

extern "C" {

struct SXCNativePickerSelection {
    uint32_t display_id;
    uint32_t left;
    uint32_t top;
    uint32_t width;
    uint32_t height;
    uint32_t parent_width;
    uint32_t parent_height;
};

// Keep these values in lockstep with the foreground Tauri Rust bridge. They
// are a tiny private C ABI, not a wire protocol or a public command result.
enum SXCNativePickerResult : int32_t {
    SXCNativePickerPicked = 0,
    SXCNativePickerCancelled = 1,
    SXCNativePickerRefused = 2,
    SXCNativePickerFailure = 3,
};

}

namespace {

constexpr CGFloat kKeyboardStep = 2.0;
constexpr CGFloat kMinimumLogicalExtent = 2.0;
constexpr double kIntegralTolerance = 0.01;

bool finite_number(double value) {
    return std::isfinite(value);
}

bool integral_u32(double value, uint32_t* output) {
    if (!output || !finite_number(value) || value < 0.0
        || value > static_cast<double>(std::numeric_limits<uint32_t>::max())) {
        return false;
    }
    const double rounded = std::round(value);
    if (std::fabs(value - rounded) > kIntegralTolerance) return false;
    *output = static_cast<uint32_t>(rounded);
    return true;
}

bool even_range(double start, double end, uint32_t limit, uint32_t* out_start, uint32_t* out_extent) {
    if (!out_start || !out_extent || !finite_number(start) || !finite_number(end)
        || start < 0.0 || end <= start || end > static_cast<double>(limit)) {
        return false;
    }
    const double floored_start = std::floor(start);
    const double ceiled_end = std::ceil(end);
    if (floored_start < 0.0 || ceiled_end > static_cast<double>(limit)
        || floored_start > static_cast<double>(std::numeric_limits<uint32_t>::max())
        || ceiled_end > static_cast<double>(std::numeric_limits<uint32_t>::max())) {
        return false;
    }
    uint64_t aligned_start = static_cast<uint64_t>(floored_start) & ~uint64_t{1};
    uint64_t aligned_end = static_cast<uint64_t>(ceiled_end);
    if (aligned_end & 1U) ++aligned_end;
    if (aligned_end <= aligned_start || aligned_end > limit || aligned_end - aligned_start < 2
        || aligned_end - aligned_start > std::numeric_limits<uint32_t>::max()) {
        return false;
    }
    *out_start = static_cast<uint32_t>(aligned_start);
    *out_extent = static_cast<uint32_t>(aligned_end - aligned_start);
    return true;
}

NSScreen* screen_for_point(NSPoint point) {
    for (NSScreen* screen in [NSScreen screens]) {
        if (NSPointInRect(point, screen.frame)) return screen;
    }
    return nil;
}

NSRect union_of_screens() {
    NSRect desktop = NSZeroRect;
    bool has_screen = false;
    for (NSScreen* screen in [NSScreen screens]) {
        desktop = has_screen ? NSUnionRect(desktop, screen.frame) : screen.frame;
        has_screen = true;
    }
    return desktop;
}

NSRect normalized_clamped_rect(NSPoint first, NSPoint second, NSRect bounds) {
    first.x = std::min(std::max(first.x, NSMinX(bounds)), NSMaxX(bounds));
    first.y = std::min(std::max(first.y, NSMinY(bounds)), NSMaxY(bounds));
    second.x = std::min(std::max(second.x, NSMinX(bounds)), NSMaxX(bounds));
    second.y = std::min(std::max(second.y, NSMinY(bounds)), NSMaxY(bounds));
    return NSMakeRect(
        std::min(first.x, second.x),
        std::min(first.y, second.y),
        std::fabs(second.x - first.x),
        std::fabs(second.y - first.y));
}

}  // namespace

@interface SXCRegionPickerPanel : NSPanel
@end

@implementation SXCRegionPickerPanel

- (BOOL)canBecomeKeyWindow { return YES; }
- (BOOL)canBecomeMainWindow { return YES; }

- (void)resignKeyWindow {
    [super resignKeyWindow];
    // A crop preview is trustworthy only while this owned modal panel retains
    // focus. Do not leave a confirmed-looking selection alive after another
    // app, Space, or panel takes focus; the Rust ticket owner receives a typed
    // refusal and cannot reserve capture from this interaction.
    if ([NSApp modalWindow] == self) {
        [NSApp stopModalWithCode:SXCNativePickerRefused];
    }
}

@end

@interface SXCRegionPickerView : NSView

- (instancetype)initWithFrame:(NSRect)frame
                  desktopFrame:(NSRect)desktop_frame
                        result:(SXCNativePickerSelection*)result;

@end

@implementation SXCRegionPickerView {
    NSRect _desktopFrame;
    NSPoint _dragAnchor;
    NSRect _selection;
    NSScreen* _selectedScreen;
    SXCNativePickerSelection* _result;
    BOOL _dragging;
    BOOL _selectionRefused;
}

- (instancetype)initWithFrame:(NSRect)frame
                  desktopFrame:(NSRect)desktop_frame
                        result:(SXCNativePickerSelection*)result {
    self = [super initWithFrame:frame];
    if (!self) return nil;
    _desktopFrame = desktop_frame;
    _result = result;
    self.wantsLayer = YES;
    return self;
}

- (BOOL)acceptsFirstResponder { return YES; }

- (NSPoint)globalPointForEvent:(NSEvent*)event {
    return [self.window convertPointToScreen:event.locationInWindow];
}

- (NSRect)localRect:(NSRect)global {
    return NSOffsetRect(global, -_desktopFrame.origin.x, -_desktopFrame.origin.y);
}

- (BOOL)hasSelection {
    return _selectedScreen && _selection.size.width >= kMinimumLogicalExtent
        && _selection.size.height >= kMinimumLogicalExtent;
}

- (void)mouseDown:(NSEvent*)event {
    NSPoint point = [self globalPointForEvent:event];
    NSScreen* screen = screen_for_point(point);
    if (!screen) {
        NSBeep();
        return;
    }
    _selectedScreen = screen;
    _dragAnchor = point;
    _selection = NSMakeRect(point.x, point.y, 0.0, 0.0);
    _dragging = YES;
    _selectionRefused = NO;
    [self setNeedsDisplay:YES];
}

- (void)mouseDragged:(NSEvent*)event {
    if (!_dragging || !_selectedScreen) return;
    _selection = normalized_clamped_rect(_dragAnchor, [self globalPointForEvent:event], _selectedScreen.frame);
    _selectionRefused = NO;
    [self setNeedsDisplay:YES];
}

- (void)mouseUp:(NSEvent*)event {
    if (!_dragging || !_selectedScreen) return;
    _selection = normalized_clamped_rect(_dragAnchor, [self globalPointForEvent:event], _selectedScreen.frame);
    _dragging = NO;
    _selectionRefused = ![self hasSelection];
    [self setNeedsDisplay:YES];
}

- (void)moveSelectionX:(CGFloat)dx y:(CGFloat)dy {
    if (![self hasSelection]) {
        NSBeep();
        return;
    }
    NSRect frame = _selectedScreen.frame;
    _selection.origin.x = std::min(
        std::max(_selection.origin.x + dx, NSMinX(frame)),
        NSMaxX(frame) - _selection.size.width);
    _selection.origin.y = std::min(
        std::max(_selection.origin.y + dy, NSMinY(frame)),
        NSMaxY(frame) - _selection.size.height);
    _selectionRefused = NO;
    [self setNeedsDisplay:YES];
}

- (void)resizeSelectionForKey:(unsigned short)keyCode {
    if (![self hasSelection]) {
        NSBeep();
        return;
    }
    NSRect frame = _selectedScreen.frame;
    switch (keyCode) {
        case 123:  // left: shrink from the right edge
            _selection.size.width = std::max(kMinimumLogicalExtent, _selection.size.width - kKeyboardStep);
            break;
        case 124:  // right: grow toward the right edge
            _selection.size.width = std::min(NSMaxX(frame) - _selection.origin.x, _selection.size.width + kKeyboardStep);
            break;
        case 125:  // down: shrink from the visual top edge
            _selection.size.height = std::max(kMinimumLogicalExtent, _selection.size.height - kKeyboardStep);
            break;
        case 126:  // up: grow toward the visual top edge
            _selection.size.height = std::min(NSMaxY(frame) - _selection.origin.y, _selection.size.height + kKeyboardStep);
            break;
        default:
            return;
    }
    _selectionRefused = NO;
    [self setNeedsDisplay:YES];
}

- (BOOL)writeResult {
    if (!_result || ![self hasSelection]) return NO;

    // `NSScreen` has a bottom-left point coordinate system, while the native
    // crop consumed by ScreenCaptureKit is top-left pixel relative to its
    // display. Converting both the screen and selection to backing space first
    // preserves asymmetric DPI and negative desktop origins without assuming a
    // global scale or a primary-display origin.
    NSRect parentBacking = [_selectedScreen convertRectToBacking:_selectedScreen.frame];
    NSRect selectedBacking = [_selectedScreen convertRectToBacking:_selection];
    const double parentMinX = std::min(NSMinX(parentBacking), NSMaxX(parentBacking));
    const double parentMinY = std::min(NSMinY(parentBacking), NSMaxY(parentBacking));
    const double parentWidth = std::fabs(parentBacking.size.width);
    const double parentHeight = std::fabs(parentBacking.size.height);
    uint32_t parentWidthPixels = 0;
    uint32_t parentHeightPixels = 0;
    if (!integral_u32(parentWidth, &parentWidthPixels) || !integral_u32(parentHeight, &parentHeightPixels)
        || parentWidthPixels < 2 || parentHeightPixels < 2) {
        return NO;
    }

    const double selectedMinX = std::min(NSMinX(selectedBacking), NSMaxX(selectedBacking));
    const double selectedMinY = std::min(NSMinY(selectedBacking), NSMaxY(selectedBacking));
    const double selectedMaxX = std::max(NSMinX(selectedBacking), NSMaxX(selectedBacking));
    const double selectedMaxY = std::max(NSMinY(selectedBacking), NSMaxY(selectedBacking));
    uint32_t left = 0;
    uint32_t width = 0;
    uint32_t top = 0;
    uint32_t height = 0;
    if (!even_range(selectedMinX - parentMinX, selectedMaxX - parentMinX, parentWidthPixels, &left, &width)
        || !even_range(
            parentHeight - (selectedMaxY - parentMinY),
            parentHeight - (selectedMinY - parentMinY),
            parentHeightPixels,
            &top,
            &height)) {
        return NO;
    }

    NSNumber* screenNumber = [[_selectedScreen deviceDescription] objectForKey:@"NSScreenNumber"];
    const unsigned long long displayNumber = screenNumber.unsignedLongLongValue;
    if (!screenNumber || displayNumber == 0 || displayNumber > std::numeric_limits<uint32_t>::max()) return NO;
    _result->display_id = static_cast<uint32_t>(displayNumber);
    _result->left = left;
    _result->top = top;
    _result->width = width;
    _result->height = height;
    _result->parent_width = parentWidthPixels;
    _result->parent_height = parentHeightPixels;
    return YES;
}

- (void)keyDown:(NSEvent*)event {
    const unsigned short keyCode = event.keyCode;
    if (keyCode == 53) {  // Escape
        [NSApp stopModalWithCode:SXCNativePickerCancelled];
        return;
    }
    if (keyCode == 36 || keyCode == 76) {  // Return / keypad Enter
        if ([self writeResult]) {
            [NSApp stopModalWithCode:SXCNativePickerPicked];
        } else {
            _selectionRefused = YES;
            NSBeep();
            [self setNeedsDisplay:YES];
        }
        return;
    }
    if (keyCode < 123 || keyCode > 126) {
        [super keyDown:event];
        return;
    }
    if ((event.modifierFlags & NSEventModifierFlagShift) != 0) {
        [self resizeSelectionForKey:keyCode];
        return;
    }
    switch (keyCode) {
        case 123: [self moveSelectionX:-kKeyboardStep y:0.0]; break;
        case 124: [self moveSelectionX:kKeyboardStep y:0.0]; break;
        case 125: [self moveSelectionX:0.0 y:-kKeyboardStep]; break;
        case 126: [self moveSelectionX:0.0 y:kKeyboardStep]; break;
        default: break;
    }
}

- (void)drawRect:(NSRect)dirtyRect {
    (void)dirtyRect;
    [[NSColor colorWithCalibratedWhite:0.0 alpha:0.48] setFill];
    NSRectFill(self.bounds);

    NSDictionary* helpAttributes = @{
        NSFontAttributeName: [NSFont systemFontOfSize:16.0 weight:NSFontWeightMedium],
        NSForegroundColorAttributeName: NSColor.whiteColor,
    };
    [@"Drag to choose part of one display  •  Enter confirms  •  Escape cancels  •  arrows move  •  Shift-arrows resize"
        drawAtPoint:NSMakePoint(28.0, NSHeight(self.bounds) - 38.0)
        withAttributes:helpAttributes];

    if (![self hasSelection]) {
        return;
    }
    NSRect local = [self localRect:_selection];
    NSRectFillUsingOperation(local, NSCompositingOperationClear);
    NSColor* border = _selectionRefused ? NSColor.systemRedColor : NSColor.systemBlueColor;
    [border setStroke];
    NSFrameRectWithWidth(local, 3.0);
    NSString* status = _selectionRefused
        ? @"This area cannot be captured at the current display scale. Resize it and press Enter."
        : @"Selection preview — press Enter to use this region";
    NSDictionary* statusAttributes = @{
        NSFontAttributeName: [NSFont systemFontOfSize:14.0 weight:NSFontWeightSemibold],
        NSForegroundColorAttributeName: border,
    };
    [status drawAtPoint:NSMakePoint(NSMinX(local), NSMaxY(local) + 8.0) withAttributes:statusAttributes];
}

@end

extern "C" int32_t sxc_macos_region_picker_is_main_thread() {
    return [NSThread isMainThread] ? 1 : 0;
}

extern "C" int32_t sxc_macos_region_picker_present(SXCNativePickerSelection* selection) {
    // AppKit is main-thread-only. cutd is intentionally never an AppKit owner;
    // the foreground Tauri shell calls this exact bridge on its AppKit main
    // thread. Refuse rather than creating a cross-thread panel or pretending
    // an abstract web overlay is a native selection.
    if (!selection || !sxc_macos_region_picker_is_main_thread()) return SXCNativePickerRefused;
    *selection = {};

    @autoreleasepool {
        NSRect desktop = union_of_screens();
        if (NSIsEmptyRect(desktop)) return SXCNativePickerRefused;
        [NSApplication sharedApplication];
        SXCRegionPickerPanel* panel = [[SXCRegionPickerPanel alloc]
            initWithContentRect:desktop
                      styleMask:NSWindowStyleMaskBorderless
                        backing:NSBackingStoreBuffered
                          defer:NO];
        if (!panel) return SXCNativePickerFailure;
        panel.opaque = NO;
        panel.backgroundColor = NSColor.clearColor;
        panel.hasShadow = NO;
        panel.level = NSModalPanelWindowLevel;
        panel.collectionBehavior = NSWindowCollectionBehaviorCanJoinAllSpaces
            | NSWindowCollectionBehaviorFullScreenAuxiliary;
        panel.releasedWhenClosed = NO;
        panel.contentView = [[SXCRegionPickerView alloc]
            initWithFrame:NSMakeRect(0.0, 0.0, desktop.size.width, desktop.size.height)
            desktopFrame:desktop
            result:selection];
        [panel makeKeyAndOrderFront:nil];
        [NSApp activateIgnoringOtherApps:YES];
        const NSInteger code = [NSApp runModalForWindow:panel];
        [panel orderOut:nil];
        if (code == SXCNativePickerPicked) return SXCNativePickerPicked;
        if (code == SXCNativePickerCancelled) return SXCNativePickerCancelled;
        return SXCNativePickerRefused;
    }
}
