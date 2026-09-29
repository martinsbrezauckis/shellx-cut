#import <AVFoundation/AVFoundation.h>
#import <CoreMedia/CoreMedia.h>
#import <Foundation/Foundation.h>
#include <atomic>
#include <cerrno>
#include <chrono>
#include <cstdio>
#include <cstring>
#include <fcntl.h>
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

// Pending -> accepting -> closed. A late didStart must never re-enable the
// callback after Stop has detached and drained its serial delivery queue.
static constexpr int kSxcCameraPending = 0;
static constexpr int kSxcCameraAccepting = 1;
static constexpr int kSxcCameraClosed = 2;

@interface SxcCameraDelegate : NSObject <AVCaptureFileOutputRecordingDelegate,
                                         AVCaptureFileOutputDelegate,
                                         AVCaptureVideoDataOutputSampleBufferDelegate> {
@public
    std::atomic_int _sampleState;
    std::atomic_uint_fast64_t _movieStartPtsNs;
    std::atomic_uint_fast64_t _movieLastPtsNs;
    uint64_t _moviePenultimatePtsNs;
    std::atomic_uint_fast64_t _movieLastDurationNs;
    std::atomic_uint_fast64_t _movieLastCadenceNs;
    std::atomic_bool _movieStopRequested;
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
        _sampleState.store(kSxcCameraPending);
        _movieStartPtsNs.store(0);
        _movieLastPtsNs.store(0);
        _moviePenultimatePtsNs = 0;
        _movieLastDurationNs.store(0);
        _movieLastCadenceNs.store(0);
        _movieStopRequested.store(false);
        _callback = callback;
        _context = context;
        _firstFrame = dispatch_semaphore_create(0);
        _finished = dispatch_semaphore_create(0);
        _deviceLost.store(false);
    }
    return self;
}

// Observe the file output's own video samples without changing when it starts
// compression. DataOutput samples are a different branch of the session graph.
- (BOOL)captureOutputShouldProvideSampleAccurateRecordingStart:(AVCaptureOutput *)output {
    (void)output;
    return NO;
}

- (void)captureOutput:(AVCaptureFileOutput *)captureOutput
 didStartRecordingToOutputFileAtURL:(NSURL *)outputFileURL
      fromConnections:(NSArray<AVCaptureConnection *> *)connections {
    (void)captureOutput;
    (void)outputFileURL;
    (void)connections;
    int expected = kSxcCameraPending;
    _sampleState.compare_exchange_strong(expected, kSxcCameraAccepting);
}

- (void)captureOutput:(AVCaptureFileOutput *)captureOutput
 didStartRecordingToOutputFileAtURL:(NSURL *)outputFileURL
             startPTS:(CMTime)startPTS
      fromConnections:(NSArray<AVCaptureConnection *> *)connections {
    _movieStartPtsNs.store(sxc_time_ns(startPTS));
    [self captureOutput:captureOutput
        didStartRecordingToOutputFileAtURL:outputFileURL
        fromConnections:connections];
}

- (void)captureOutput:(AVCaptureOutput *)output
 didOutputSampleBuffer:(CMSampleBufferRef)sampleBuffer
       fromConnection:(AVCaptureConnection *)connection {
    (void)connection;
    if ([output isKindOfClass:[AVCaptureMovieFileOutput class]]) {
        CMFormatDescriptionRef format = CMSampleBufferGetFormatDescription(sampleBuffer);
        if (!format || CMFormatDescriptionGetMediaType(format) != kCMMediaType_Video) return;
        const uint64_t pts = sxc_time_ns(CMSampleBufferGetPresentationTimeStamp(sampleBuffer));
        const uint64_t duration = sxc_time_ns(CMSampleBufferGetDuration(sampleBuffer));
        if (pts == 0 || duration > UINT64_MAX - pts) return;
        // File-output callbacks can include warmup samples. Keep the two
        // greatest presentation timestamps, regardless of arrival order, so
        // the native final sample names the same timeline endpoint as the
        // encoded packet proof. A later unencoded sample still fails that proof.
        @synchronized (self) {
            const uint64_t greatest = _movieLastPtsNs.load();
            if (pts > greatest) {
                _moviePenultimatePtsNs = greatest;
                _movieLastPtsNs.store(pts);
                _movieLastDurationNs.store(duration);
            } else if (pts < greatest && pts > _moviePenultimatePtsNs) {
                _moviePenultimatePtsNs = pts;
            }
            if (_moviePenultimatePtsNs > 0) {
                _movieLastCadenceNs.store(_movieLastPtsNs.load() - _moviePenultimatePtsNs);
            }
        }
        // AVFoundation guarantees that stopRecording called from this
        // callback includes samples before the current sample. The current
        // sample can remain outside the presented movie edit range.
        // A Stop request arriving from Rust is therefore completed at the
        // file-output frame boundary instead of racing the writer from a
        // different thread and leaving several encoded frames past the last
        // timing callback.
        if (_movieStopRequested.exchange(false)) {
            [(AVCaptureMovieFileOutput *)output stopRecording];
        }
        return;
    }
    if (_sampleState.load() != kSxcCameraAccepting || !_callback) return;
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
    if (error && ![error.userInfo[AVErrorRecordingSuccessfullyFinishedKey] boolValue]) {
        _terminalError = error;
    }
    _sampleState.store(kSxcCameraClosed);
    dispatch_semaphore_signal(_finished);
}

- (void)captureDeviceDisconnected:(NSNotification *)notification {
    (void)notification;
    _deviceLost.store(true);
    _sampleState.store(kSxcCameraClosed);
    dispatch_semaphore_signal(_firstFrame);
}

- (void)captureSessionRuntimeError:(NSNotification *)notification {
    (void)notification;
    // Treat a terminal AVFoundation runtime error as device loss so the
    // independently finalized take can never be reported Complete.
    _deviceLost.store(true);
    _sampleState.store(kSxcCameraClosed);
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
    handle.delegate->_sampleState.store(kSxcCameraClosed);
    [handle.samples setSampleBufferDelegate:nil queue:nil];
    dispatch_sync(handle.queue, ^{});
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
        handle.movie.delegate = handle.delegate;
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
        // didStartRecording admits samples only after the asynchronous movie
        // writer has started; callbacks before it are warm-up observations.
        if (!sxc_wait_camera_event(handle.delegate->_firstFrame, 5) ||
            handle.delegate->_deviceLost.load()) {
            sxc_quiesce_samples(handle);
            if (handle.movie.isRecording) [handle.movie stopRecording];
            sxc_wait_camera_event(handle.delegate->_finished, 10);
            [handle.session stopRunning];
            handle.movie.delegate = nil;
            [[NSNotificationCenter defaultCenter] removeObserver:handle.delegate];
            sxc_error(error, error_capacity, handle.delegate->_deviceLost.load()
                ? @"The selected camera was disconnected before its first frame"
                : @"AVFoundation started without a camera frame");
            return nullptr;
        }
        return (__bridge_retained void *)handle;
    }
}

extern "C" int32_t sxc_macos_camera_stop(void *opaque, char *error, size_t error_capacity,
                                            int32_t *device_lost, uint64_t *movie_start_pts_ns,
                                            uint64_t *movie_last_pts_ns,
                                            uint64_t *movie_last_duration_ns,
                                            uint64_t *movie_last_cadence_ns) {
    @autoreleasepool {
        if (device_lost) *device_lost = 0;
        if (movie_start_pts_ns) *movie_start_pts_ns = 0;
        if (movie_last_pts_ns) *movie_last_pts_ns = 0;
        if (movie_last_duration_ns) *movie_last_duration_ns = 0;
        if (movie_last_cadence_ns) *movie_last_cadence_ns = 0;
        if (!opaque) {
            sxc_error(error, error_capacity, @"Camera owner is missing");
            return -1;
        }
        SxcCameraHandle *handle = (__bridge_transfer SxcCameraHandle *)opaque;
        const bool movie_was_recording = handle.movie.isRecording;
        if (movie_was_recording) {
            // The file-output callback consumes this request after recording
            // the current sample. This is the documented sample-accurate
            // Stop route for AVCaptureFileOutputDelegate.
            handle.delegate->_movieStopRequested.store(true);
        }
        sxc_quiesce_samples(handle);
        // didFinish is required for every recording request, including one
        // that stopped before isRecording could still report true.
        if (!sxc_wait_camera_event(handle.delegate->_finished, 15)) {
            // A broken callback delivery path must still settle the native
            // owner before returning the existing actionable error.
            handle.delegate->_movieStopRequested.store(false);
            if (handle.movie.isRecording) [handle.movie stopRecording];
            [handle.session stopRunning];
            handle.movie.delegate = nil;
            sxc_error(error, error_capacity, @"AVFoundation did not finish the camera recording");
            return -1;
        }
        [handle.session stopRunning];
        handle.movie.delegate = nil;
        const bool lost = handle.delegate->_deviceLost.load();
        if (device_lost) *device_lost = lost ? 1 : 0;
        [[NSNotificationCenter defaultCenter] removeObserver:handle.delegate];
        // The regular no-replace finalizer independently rejects a damaged
        // movie. If it is valid after a disconnect, preserve it as DeviceLost.
        if (handle.delegate->_terminalError && !lost) {
            sxc_error(error, error_capacity, handle.delegate->_terminalError.localizedDescription);
            return -1;
        }
        @synchronized (handle.delegate) {
            if (movie_start_pts_ns) *movie_start_pts_ns = handle.delegate->_movieStartPtsNs.load();
            if (movie_last_pts_ns) *movie_last_pts_ns = handle.delegate->_movieLastPtsNs.load();
            if (movie_last_duration_ns) *movie_last_duration_ns = handle.delegate->_movieLastDurationNs.load();
            if (movie_last_cadence_ns) *movie_last_cadence_ns = handle.delegate->_movieLastCadenceNs.load();
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
