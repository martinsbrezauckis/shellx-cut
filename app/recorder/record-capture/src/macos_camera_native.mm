#import <AVFoundation/AVFoundation.h>
#import <CoreMedia/CoreMedia.h>
#import <Foundation/Foundation.h>
#import <os/log.h>
#include <atomic>
#include <cerrno>
#include <chrono>
#include <cstdio>
#include <cstring>
#include <fcntl.h>
#include <pthread.h>
#include <sys/attr.h>

typedef void (*SxcCameraFrameCallback)(void *context, uint64_t pts_ns, uint64_t duration_ns);

static void sxc_error(char *buffer, size_t capacity, NSString *message) {
    if (!buffer || capacity == 0) return;
    const char *value = message ? message.UTF8String : "unknown AVFoundation camera error";
    std::snprintf(buffer, capacity, "%s", value ?: "unknown AVFoundation camera error");
}

static uint64_t sxc_time_ns(CMTime value) {
    if (!CMTIME_IS_NUMERIC(value) || value.timescale <= 0 || value.value < 0) return 0;
    long double nanos = (long double)value.value * 1000000000.0L / (long double)value.timescale;
    if (nanos <= 0 || nanos > (long double)UINT64_MAX) return 0;
    return (uint64_t)nanos;
}

static void sxc_camera_diag(const char *event) {
    uint64_t thread_id = 0;
    pthread_threadid_np(nullptr, &thread_id);
    os_log_error(OS_LOG_DEFAULT, "SXCCameraDiag %{public}s thread=%llu main=%{public}s",
                 event, (unsigned long long)thread_id, [NSThread isMainThread] ? "yes" : "no");
}

static bool sxc_wait_camera_event(dispatch_semaphore_t event, int timeout_seconds) {
    // AVCaptureFileOutput may deliver its recording delegate through the
    // initiating thread's run loop. Blocking that thread on a semaphore can
    // prevent didFinish from running, then make stopRunning wait for the same
    // thread. Service its default mode while preserving the existing deadline.
    const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(timeout_seconds);
    for (;;) {
        if (dispatch_semaphore_wait(event, DISPATCH_TIME_NOW) == 0) return true;
        if (std::chrono::steady_clock::now() >= deadline) return false;
        @autoreleasepool {
            [[NSRunLoop currentRunLoop] runMode:NSDefaultRunLoopMode
                                    beforeDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];
        }
        if (dispatch_semaphore_wait(event,
                                    dispatch_time(DISPATCH_TIME_NOW, 10 * NSEC_PER_MSEC)) == 0) {
            return true;
        }
    }
}

@interface SxcCameraDelegate : NSObject <AVCaptureFileOutputRecordingDelegate,
                                         AVCaptureVideoDataOutputSampleBufferDelegate> {
@public
    std::atomic_bool _accepting;
    SxcCameraFrameCallback _callback;
    void *_context;
    dispatch_semaphore_t _firstFrame;
    dispatch_semaphore_t _finished;
    NSError *_terminalError;
    std::atomic_bool _deviceLost;
}
- (instancetype)initWithCallback:(SxcCameraFrameCallback)callback context:(void *)context;
@end

@implementation SxcCameraDelegate
- (instancetype)initWithCallback:(SxcCameraFrameCallback)callback context:(void *)context {
    self = [super init];
    if (self) {
        _accepting.store(false);
        _callback = callback;
        _context = context;
        _firstFrame = dispatch_semaphore_create(0);
        _finished = dispatch_semaphore_create(0);
        _deviceLost.store(false);
    }
    return self;
}

- (void)captureOutput:(AVCaptureOutput *)output
 didOutputSampleBuffer:(CMSampleBufferRef)sampleBuffer
       fromConnection:(AVCaptureConnection *)connection {
    (void)output;
    (void)connection;
    if (!_accepting.load() || !_callback) return;
    uint64_t pts = sxc_time_ns(CMSampleBufferGetPresentationTimeStamp(sampleBuffer));
    uint64_t duration = sxc_time_ns(CMSampleBufferGetDuration(sampleBuffer));
    if (duration == 0) duration = 33333333;
    _callback(_context, pts, duration);
    dispatch_semaphore_signal(_firstFrame);
}

- (void)captureOutput:(AVCaptureFileOutput *)captureOutput
 didFinishRecordingToOutputFileAtURL:(NSURL *)outputFileURL
      fromConnections:(NSArray<AVCaptureConnection *> *)connections
                error:(NSError *)error {
    (void)captureOutput;
    (void)outputFileURL;
    (void)connections;
    sxc_camera_diag("didFinishRecording entered");
    if (error && ![error.userInfo[AVErrorRecordingSuccessfullyFinishedKey] boolValue]) {
        _terminalError = error;
    }
    _accepting.store(false);
    dispatch_semaphore_signal(_finished);
    sxc_camera_diag("didFinishRecording signaled");
}

- (void)captureDeviceDisconnected:(NSNotification *)notification {
    (void)notification;
    _deviceLost.store(true);
    _accepting.store(false);
    dispatch_semaphore_signal(_firstFrame);
}

- (void)captureSessionRuntimeError:(NSNotification *)notification {
    (void)notification;
    // Treat a terminal AVFoundation runtime error as device loss so the
    // independently finalized take can never be reported Complete.
    _deviceLost.store(true);
    _accepting.store(false);
    dispatch_semaphore_signal(_firstFrame);
}
@end

@interface SxcCameraHandle : NSObject
@property(nonatomic, strong) AVCaptureSession *session;
@property(nonatomic, strong) AVCaptureMovieFileOutput *movie;
@property(nonatomic, strong) AVCaptureVideoDataOutput *samples;
@property(nonatomic, strong) SxcCameraDelegate *delegate;
@property(nonatomic, strong) dispatch_queue_t queue;
@end
@implementation SxcCameraHandle
@end

static void sxc_quiesce_samples(SxcCameraHandle *handle) {
    sxc_camera_diag("sample quiescence begin");
    handle.delegate->_accepting.store(false);
    [handle.samples setSampleBufferDelegate:nil queue:nil];
    dispatch_sync(handle.queue, ^{});
    sxc_camera_diag("sample quiescence drained");
}

extern "C" size_t sxc_macos_camera_devices_json(char *buffer, size_t capacity) {
    @autoreleasepool {
        NSMutableArray *rows = [NSMutableArray array];
        for (AVCaptureDevice *device in [AVCaptureDevice devicesWithMediaType:AVMediaTypeVideo]) {
            if (!device.uniqueID.length) continue;
            [rows addObject:@{
                @"uid": device.uniqueID,
                @"label": device.localizedName.length ? device.localizedName : @"Camera"
            }];
        }
        NSError *error = nil;
        NSData *data = [NSJSONSerialization dataWithJSONObject:rows options:0 error:&error];
        if (!data || error) return 0;
        size_t required = data.length + 1;
        if (buffer && capacity >= required) {
            std::memcpy(buffer, data.bytes, data.length);
            buffer[data.length] = '\0';
        }
        return required;
    }
}

extern "C" int32_t sxc_macos_camera_authorization(void) {
    switch ([AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeVideo]) {
        case AVAuthorizationStatusAuthorized: return 2;
        case AVAuthorizationStatusDenied: return 1;
        case AVAuthorizationStatusRestricted: return 3;
        case AVAuthorizationStatusNotDetermined: return 0;
    }
    return 3;
}

static bool sxc_authorize(char *error, size_t error_capacity) {
    AVAuthorizationStatus status = [AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeVideo];
    if (status == AVAuthorizationStatusAuthorized) return true;
    if (status != AVAuthorizationStatusNotDetermined) {
        sxc_error(error, error_capacity, @"Camera access is denied in System Settings");
        return false;
    }
    dispatch_semaphore_t settled = dispatch_semaphore_create(0);
    __block BOOL granted = NO;
    [AVCaptureDevice requestAccessForMediaType:AVMediaTypeVideo completionHandler:^(BOOL allowed) {
        granted = allowed;
        dispatch_semaphore_signal(settled);
    }];
    if (dispatch_semaphore_wait(settled, dispatch_time(DISPATCH_TIME_NOW, 30 * NSEC_PER_SEC)) != 0 || !granted) {
        sxc_error(error, error_capacity, @"Camera permission was not granted");
        return false;
    }
    return true;
}

extern "C" void *sxc_macos_camera_start(const char *device_uid,
                                          const char *output_path,
                                          SxcCameraFrameCallback callback,
                                          void *context,
                                          char *error,
                                          size_t error_capacity) {
    @autoreleasepool {
        if (!device_uid || !output_path || !callback || !sxc_authorize(error, error_capacity)) return nullptr;
        NSString *uid = [NSString stringWithUTF8String:device_uid];
        NSString *path = [NSString stringWithUTF8String:output_path];
        if (!uid.length || !path.length) {
            sxc_error(error, error_capacity, @"Camera request is not valid UTF-8");
            return nullptr;
        }
        AVCaptureDevice *selected = nil;
        for (AVCaptureDevice *device in [AVCaptureDevice devicesWithMediaType:AVMediaTypeVideo]) {
            if ([device.uniqueID isEqualToString:uid]) { selected = device; break; }
        }
        if (!selected) {
            sxc_error(error, error_capacity, @"The selected camera is no longer connected");
            return nullptr;
        }
        NSError *inputError = nil;
        AVCaptureDeviceInput *input = [AVCaptureDeviceInput deviceInputWithDevice:selected error:&inputError];
        if (!input) {
            sxc_error(error, error_capacity, inputError.localizedDescription);
            return nullptr;
        }
        SxcCameraHandle *handle = [SxcCameraHandle new];
        handle.session = [AVCaptureSession new];
        handle.movie = [AVCaptureMovieFileOutput new];
        handle.samples = [AVCaptureVideoDataOutput new];
        handle.samples.alwaysDiscardsLateVideoFrames = NO;
        handle.delegate = [[SxcCameraDelegate alloc] initWithCallback:callback context:context];
        handle.queue = dispatch_queue_create("com.theshellx.cut.camera.frames", DISPATCH_QUEUE_SERIAL);
        [handle.samples setSampleBufferDelegate:handle.delegate queue:handle.queue];
        [handle.session beginConfiguration];
        if ([handle.session canSetSessionPreset:AVCaptureSessionPresetHigh]) {
            handle.session.sessionPreset = AVCaptureSessionPresetHigh;
        }
        if (![handle.session canAddInput:input] || ![handle.session canAddOutput:handle.movie] ||
            ![handle.session canAddOutput:handle.samples]) {
            [handle.session commitConfiguration];
            sxc_error(error, error_capacity, @"The selected camera cannot provide a recordable video stream");
            return nullptr;
        }
        [handle.session addInput:input];
        [handle.session addOutput:handle.movie];
        [handle.session addOutput:handle.samples];
        [handle.session commitConfiguration];
        NSNotificationCenter *notifications = [NSNotificationCenter defaultCenter];
        [notifications addObserver:handle.delegate selector:@selector(captureDeviceDisconnected:)
                               name:AVCaptureDeviceWasDisconnectedNotification object:selected];
        [notifications addObserver:handle.delegate selector:@selector(captureSessionRuntimeError:)
                               name:AVCaptureSessionRuntimeErrorNotification object:handle.session];
        [handle.session startRunning];
        if (!handle.session.isRunning) {
            sxc_error(error, error_capacity, @"AVFoundation did not start the selected camera");
            return nullptr;
        }
        NSURL *url = [NSURL fileURLWithPath:path];
        [handle.movie startRecordingToOutputFileURL:url recordingDelegate:handle.delegate];
        sxc_camera_diag("startRecording requested");
        // Samples delivered while the session was merely warming up must not
        // enter the shared screen clock. Admit callbacks only after the movie
        // writer owns its destination.
        handle.delegate->_accepting.store(true);
        if (!sxc_wait_camera_event(handle.delegate->_firstFrame, 5) ||
            handle.delegate->_deviceLost.load()) {
            sxc_quiesce_samples(handle);
            if (handle.movie.isRecording) [handle.movie stopRecording];
            sxc_camera_diag("failed Start stopRecording requested");
            sxc_wait_camera_event(handle.delegate->_finished, 10);
            sxc_camera_diag("failed Start stopRunning enter");
            [handle.session stopRunning];
            sxc_camera_diag("failed Start stopRunning exit");
            [[NSNotificationCenter defaultCenter] removeObserver:handle.delegate];
            sxc_error(error, error_capacity, handle.delegate->_deviceLost.load()
                ? @"The selected camera was disconnected before its first frame"
                : @"AVFoundation started without a camera frame");
            return nullptr;
        }
        sxc_camera_diag("first frame admitted");
        return (__bridge_retained void *)handle;
    }
}

extern "C" int32_t sxc_macos_camera_stop(void *opaque, char *error, size_t error_capacity,
                                            int32_t *device_lost) {
    @autoreleasepool {
        if (device_lost) *device_lost = 0;
        if (!opaque) {
            sxc_error(error, error_capacity, @"Camera owner is missing");
            return -1;
        }
        SxcCameraHandle *handle = (__bridge_transfer SxcCameraHandle *)opaque;
        sxc_camera_diag("Stop entered");
        sxc_quiesce_samples(handle);
        if (handle.movie.isRecording) {
            [handle.movie stopRecording];
        }
        sxc_camera_diag("stopRecording requested");
        // didFinish is required for every recording request, including one
        // that stopped before isRecording could still report true.
        if (!sxc_wait_camera_event(handle.delegate->_finished, 15)) {
            sxc_camera_diag("didFinish timeout; stopRunning enter");
            [handle.session stopRunning];
            sxc_camera_diag("didFinish timeout; stopRunning exit");
            sxc_error(error, error_capacity, @"AVFoundation did not finish the camera recording");
            return -1;
        }
        sxc_camera_diag("didFinish observed; stopRunning enter");
        [handle.session stopRunning];
        sxc_camera_diag("stopRunning exit");
        const bool lost = handle.delegate->_deviceLost.load();
        if (device_lost) *device_lost = lost ? 1 : 0;
        [[NSNotificationCenter defaultCenter] removeObserver:handle.delegate];
        // The regular no-replace finalizer independently rejects a damaged
        // movie. If it is valid after a disconnect, preserve it as DeviceLost.
        if (handle.delegate->_terminalError && !lost) {
            sxc_error(error, error_capacity, handle.delegate->_terminalError.localizedDescription);
            return -1;
        }
        return 0;
    }
}

extern "C" int32_t sxc_macos_camera_publish_no_replace(const char *source,
                                                          const char *destination) {
    if (!source || !destination) return EINVAL;
    if (renameatx_np(AT_FDCWD, source, AT_FDCWD, destination, RENAME_EXCL) == 0) return 0;
    return errno ? errno : EIO;
}
