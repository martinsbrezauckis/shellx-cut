/* Explicit-use V4L2 camera owner. No function here is called by passive Doctor. */
#define _POSIX_C_SOURCE 200809L
#include <gst/gst.h>
#include <gst/gstsystemclock.h>
#include <glib-object.h>
#include <linux/videodev2.h>
#include <sys/ioctl.h>
#include <sys/stat.h>
#include <time.h>
#include <string.h>

typedef struct { guint64 start, end; } SxcInterval;
typedef struct {
    GstElement *pipeline, *source;
    GstPad *pad;
    gulong probe_id;
    GstClock *clock;
    GMutex mutex;
    GArray *intervals;
    GstSegment segment;
    gboolean has_segment, opened, bad, stopped;
    dev_t expected_rdev;
    ino_t expected_inode;
    guint width, height, fps_num, fps_den;
    char diagnosis[256];
} SxcCameraRun;

static void fail(SxcCameraRun *run) {
    g_mutex_lock(&run->mutex);
    run->bad = TRUE;
    g_mutex_unlock(&run->mutex);
}

static void bus_diagnosis(SxcCameraRun *run, GstMessage *message) {
    if (!message || GST_MESSAGE_TYPE(message) != GST_MESSAGE_ERROR) return;
    GError *error = NULL;
    gchar *debug = NULL;
    gst_message_parse_error(message, &error, &debug);
    if (error && error->message) g_strlcpy(run->diagnosis, error->message, sizeof run->diagnosis);
    g_clear_error(&error);
    g_free(debug);
}

int sxc_linux_camera_validate_fd(gint fd, guint64 expected_rdev, guint64 expected_inode) {
    struct stat st;
    struct v4l2_capability cap;
    memset(&cap, 0, sizeof cap);
    if (fstat(fd, &st) != 0 || !S_ISCHR(st.st_mode) ||
        st.st_rdev != (dev_t)expected_rdev || st.st_ino != (ino_t)expected_inode ||
        ioctl(fd, VIDIOC_QUERYCAP, &cap) != 0 ||
        !((cap.capabilities & V4L2_CAP_DEVICE_CAPS ? cap.device_caps : cap.capabilities)
          & V4L2_CAP_VIDEO_CAPTURE)) return 0;
    return 1;
}

static void prepare_format(GstElement *source, gint fd, GstCaps *caps, gpointer user) {
    (void)source; (void)caps;
    SxcCameraRun *run = user;
    if (!sxc_linux_camera_validate_fd(fd, run->expected_rdev, run->expected_inode)) {
        fail(run);
        return;
    }
    g_mutex_lock(&run->mutex);
    run->opened = TRUE;
    g_mutex_unlock(&run->mutex);
}

static GstPadProbeReturn observe(GstPad *pad, GstPadProbeInfo *info, gpointer user) {
    SxcCameraRun *run = user;
    if (GST_PAD_PROBE_INFO_TYPE(info) & GST_PAD_PROBE_TYPE_EVENT_DOWNSTREAM) {
        GstEvent *event = GST_PAD_PROBE_INFO_EVENT(info);
        if (GST_EVENT_TYPE(event) == GST_EVENT_SEGMENT) {
            const GstSegment *segment = NULL;
            gst_event_parse_segment(event, &segment);
            g_mutex_lock(&run->mutex);
            if (!segment || segment->format != GST_FORMAT_TIME || segment->rate != 1.0)
                run->bad = TRUE;
            else {
                gst_segment_copy_into(segment, &run->segment);
                run->has_segment = TRUE;
            }
            g_mutex_unlock(&run->mutex);
        }
        return GST_PAD_PROBE_OK;
    }
    if (!(GST_PAD_PROBE_INFO_TYPE(info) & GST_PAD_PROBE_TYPE_BUFFER))
        return GST_PAD_PROBE_OK;
    GstBuffer *buffer = GST_PAD_PROBE_INFO_BUFFER(info);
    g_mutex_lock(&run->mutex);
    if (run->bad || !run->opened || !run->has_segment || !buffer ||
        !GST_CLOCK_TIME_IS_VALID(GST_BUFFER_PTS(buffer)) ||
        !GST_CLOCK_TIME_IS_VALID(GST_BUFFER_DURATION(buffer)) ||
        GST_BUFFER_DURATION(buffer) == 0 || run->intervals->len >= 1000000) {
        run->bad = TRUE;
        g_mutex_unlock(&run->mutex);
        return GST_PAD_PROBE_DROP;
    }
    guint64 running = gst_segment_to_running_time(&run->segment, GST_FORMAT_TIME,
                                                   GST_BUFFER_PTS(buffer));
    guint64 base = gst_element_get_base_time(run->pipeline);
    if (!GST_CLOCK_TIME_IS_VALID(running) || !GST_CLOCK_TIME_IS_VALID(base) ||
        running > G_MAXUINT64 - base ||
        GST_BUFFER_DURATION(buffer) > G_MAXUINT64 - (running + base)) {
        run->bad = TRUE;
        g_mutex_unlock(&run->mutex);
        return GST_PAD_PROBE_DROP;
    }
    SxcInterval sample = { base + running, base + running + GST_BUFFER_DURATION(buffer) };
    /* Some V4L2 devices repeat their first PTS. Never encode the duplicate;
       later increasing native timestamps remain eligible. Regressions fail. */
    if (run->intervals->len) {
        guint64 previous = g_array_index(run->intervals, SxcInterval,
                                         run->intervals->len - 1).start;
        if (sample.start <= previous) {
            if (sample.start < previous) run->bad = TRUE;
            g_mutex_unlock(&run->mutex);
            return GST_PAD_PROBE_DROP;
        }
    }
    if (!run->width) {
        GstCaps *caps = gst_pad_get_current_caps(pad);
        const GstStructure *shape = caps ? gst_caps_get_structure(caps, 0) : NULL;
        gint w = 0, h = 0, n = 0, d = 0;
        if (!shape || !gst_structure_get_int(shape, "width", &w) ||
            !gst_structure_get_int(shape, "height", &h) ||
            !gst_structure_get_fraction(shape, "framerate", &n, &d) ||
            w <= 0 || h <= 0 || n <= 0 || d <= 0) run->bad = TRUE;
        else { run->width = w; run->height = h; run->fps_num = n; run->fps_den = d; }
        if (caps) gst_caps_unref(caps);
    }
    if (!run->bad) g_array_append_val(run->intervals, sample);
    gboolean bad = run->bad;
    g_mutex_unlock(&run->mutex);
    return bad ? GST_PAD_PROBE_DROP : GST_PAD_PROBE_OK;
}

static void release_run(SxcCameraRun *run) {
    if (!run) return;
    if (run->pad && run->probe_id) gst_pad_remove_probe(run->pad, run->probe_id);
    if (run->source) g_signal_handlers_disconnect_by_data(run->source, run);
    if (run->pad) gst_object_unref(run->pad);
    if (run->pipeline) gst_object_unref(run->pipeline);
    if (run->clock) gst_object_unref(run->clock);
    if (run->intervals) g_array_free(run->intervals, TRUE);
    g_mutex_clear(&run->mutex);
    g_free(run);
}

/* Returns only an owned, running pipeline. The caller must invoke stop before free. */
SxcCameraRun *sxc_linux_camera_start(const char *device, guint64 rdev, guint64 inode,
                                     const char *output, char *reason, guint reason_size) {
    if (reason && reason_size) reason[0] = '\0';
    if (!device || !output || !gst_init_check(NULL, NULL, NULL)) return NULL;
    SxcCameraRun *run = g_new0(SxcCameraRun, 1);
    g_mutex_init(&run->mutex);
    run->intervals = g_array_new(FALSE, FALSE, sizeof(SxcInterval));
    gst_segment_init(&run->segment, GST_FORMAT_TIME);
    run->expected_rdev = (dev_t)rdev;
    run->expected_inode = (ino_t)inode;
    run->pipeline = gst_pipeline_new("cut-camera");
    run->source = gst_element_factory_make("v4l2src", "selected-camera");
    GstElement *raw = gst_element_factory_make("capsfilter", NULL);
    GstElement *convert = gst_element_factory_make("videoconvert", NULL);
    GstElement *encoder = gst_element_factory_make("x264enc", NULL);
    GstElement *parse = gst_element_factory_make("h264parse", NULL);
    GstElement *mux = gst_element_factory_make("mp4mux", NULL);
    GstElement *sink = gst_element_factory_make("filesink", NULL);
    gboolean added = FALSE;
    if (!run->pipeline || !run->source || !raw || !convert || !encoder || !parse || !mux || !sink)
        goto fail_start;
    GstCaps *caps = gst_caps_from_string("video/x-raw");
    g_object_set(raw, "caps", caps, NULL);
    gst_caps_unref(caps);
    g_object_set(run->source, "device", device, NULL);
    g_object_set(encoder, "tune", 0x4 /* zerolatency */, "bframes", 0,
                 "key-int-max", 30, NULL);
    g_object_set(sink, "location", output, "sync", FALSE, NULL);
    g_signal_connect(run->source, "prepare-format", G_CALLBACK(prepare_format), run);
    gst_bin_add_many(GST_BIN(run->pipeline), run->source, raw, convert, encoder, parse, mux, sink, NULL);
    added = TRUE;
    if (!gst_element_link_many(run->source, raw, convert, encoder, parse, mux, sink, NULL)) goto fail_start;
    run->pad = gst_element_get_static_pad(convert, "src");
    if (!run->pad) goto fail_start;
    run->probe_id = gst_pad_add_probe(run->pad,
        GST_PAD_PROBE_TYPE_BUFFER | GST_PAD_PROBE_TYPE_EVENT_DOWNSTREAM, observe, run, NULL);
    if (!run->probe_id) goto fail_start;
    run->clock = g_object_new(GST_TYPE_SYSTEM_CLOCK, "clock-type", GST_CLOCK_TYPE_MONOTONIC, NULL);
    if (!run->clock) goto fail_start;
    gst_pipeline_use_clock(GST_PIPELINE(run->pipeline), run->clock);
    if (gst_element_set_state(run->pipeline, GST_STATE_PLAYING) == GST_STATE_CHANGE_FAILURE)
        goto fail_start;
    return run;
fail_start:
    if (run->pipeline) {
        GstBus *bus = gst_element_get_bus(run->pipeline);
        if (bus) {
            GstMessage *message = gst_bus_pop_filtered(bus, GST_MESSAGE_ERROR);
            bus_diagnosis(run, message);
            if (message) gst_message_unref(message);
            gst_object_unref(bus);
        }
    }
    if (reason && reason_size) g_strlcpy(reason, run->diagnosis, reason_size);
    if (!added) {
        if (raw) gst_object_unref(raw);
        if (convert) gst_object_unref(convert);
        if (encoder) gst_object_unref(encoder);
        if (parse) gst_object_unref(parse);
        if (mux) gst_object_unref(mux);
        if (sink) gst_object_unref(sink);
        if (run->source) { gst_object_unref(run->source); run->source = NULL; }
    }
    if (run->pipeline && gst_element_set_state(run->pipeline, GST_STATE_NULL) == GST_STATE_CHANGE_FAILURE) {
        fail(run);
        return run; /* Rust retains callback and writer ownership on failed shutdown. */
    }
    release_run(run);
    return NULL;
}

const char *sxc_linux_camera_missing_plugin(void) {
    if (!gst_init_check(NULL, NULL, NULL)) return "GStreamer initialization";
    const char *names[] = { "v4l2src", "capsfilter", "videoconvert", "x264enc",
                            "h264parse", "mp4mux", "filesink" };
    for (guint i = 0; i < G_N_ELEMENTS(names); ++i) {
        GstElementFactory *factory = gst_element_factory_find(names[i]);
        if (!factory) return names[i];
        gst_object_unref(factory);
    }
    return NULL;
}

/* A bounded wait observes admitted native samples, never file growth or arrival time. */
int sxc_linux_camera_first(SxcCameraRun *run, guint timeout_ms) {
    if (!run) return -1;
    GstBus *bus = gst_element_get_bus(run->pipeline);
    guint elapsed = 0;
    while (elapsed <= timeout_ms) {
        GstMessage *message = gst_bus_timed_pop_filtered(bus, 10 * GST_MSECOND,
            GST_MESSAGE_ERROR | GST_MESSAGE_EOS);
        if (message) {
            bus_diagnosis(run, message);
            gst_message_unref(message);
            gst_object_unref(bus);
            return -1;
        }
        g_mutex_lock(&run->mutex);
        gboolean ready = run->intervals->len > 0 && !run->bad && run->opened;
        gboolean bad = run->bad;
        g_mutex_unlock(&run->mutex);
        if (ready) { gst_object_unref(bus); return 0; }
        if (bad) { gst_object_unref(bus); return -1; }
        elapsed += 10;
    }
    gst_object_unref(bus);
    g_mutex_lock(&run->mutex);
    gboolean opened = run->opened;
    g_mutex_unlock(&run->mutex);
    return opened ? 1 : 2;
}

/* A fully stalled stream must not seal as one frame after a long recording.
   Call under the observation mutex at Stop request, before EOS can deliver
   further buffers and encoder drain consumes additional time. */
static gboolean fresh_last_interval(SxcCameraRun *run, GstClockTime stop_time,
                                    guint max_frame_gap_ms) {
    if (!GST_CLOCK_TIME_IS_VALID(stop_time) || !run->intervals->len) return FALSE;
    SxcInterval last = g_array_index(run->intervals, SxcInterval,
                                     run->intervals->len - 1);
    return last.start <= stop_time && last.end >= last.start &&
        (last.end >= stop_time || stop_time - last.end <=
         (GstClockTime)max_frame_gap_ms * GST_MSECOND);
}

/* On failure the caller must retain this pointer; no callback/userdata is released. */
int sxc_linux_camera_stop(SxcCameraRun *run, guint timeout_ms, guint max_frame_gap_ms) {
    if (!run || run->stopped) return -1;
    g_mutex_lock(&run->mutex);
    GstClockTime stop_time = run->clock ? gst_clock_get_time(run->clock) : GST_CLOCK_TIME_NONE;
    gboolean fresh_at_stop = fresh_last_interval(run, stop_time, max_frame_gap_ms);
    g_mutex_unlock(&run->mutex);
    GstBus *bus = gst_element_get_bus(run->pipeline);
    gboolean sent = gst_element_send_event(run->pipeline, gst_event_new_eos());
    GstMessage *message = sent ? gst_bus_timed_pop_filtered(bus,
        (GstClockTime)timeout_ms * GST_MSECOND, GST_MESSAGE_ERROR | GST_MESSAGE_EOS) : NULL;
    gboolean eos = message && GST_MESSAGE_TYPE(message) == GST_MESSAGE_EOS;
    bus_diagnosis(run, message);
    if (message) gst_message_unref(message);
    gst_object_unref(bus);
    if (gst_element_set_state(run->pipeline, GST_STATE_NULL) == GST_STATE_CHANGE_FAILURE)
        return -1;
    run->stopped = TRUE;
    if (run->pad && run->probe_id) { gst_pad_remove_probe(run->pad, run->probe_id); run->probe_id = 0; }
    if (run->source) g_signal_handlers_disconnect_by_data(run->source, run);
    g_mutex_lock(&run->mutex);
    gboolean valid = run->opened && !run->bad && fresh_at_stop;
    g_mutex_unlock(&run->mutex);
    return eos && valid ? 0 : -1;
}

int sxc_linux_camera_retired(SxcCameraRun *run) { return run && run->stopped; }

const char *sxc_linux_camera_diagnosis(SxcCameraRun *run) {
    return run ? run->diagnosis : "";
}

void sxc_linux_camera_free(SxcCameraRun *run) {
    if (run && run->stopped) release_run(run);
}

guint sxc_linux_camera_count(SxcCameraRun *run) {
    return run && run->stopped ? run->intervals->len : 0;
}

int sxc_linux_camera_interval(SxcCameraRun *run, guint index, guint64 *start, guint64 *end) {
    if (!run || !run->stopped || index >= run->intervals->len) return -1;
    SxcInterval value = g_array_index(run->intervals, SxcInterval, index);
    *start = value.start;
    *end = value.end;
    return 0;
}

int sxc_linux_camera_shape(SxcCameraRun *run, guint *w, guint *h, guint *n, guint *d) {
    if (!run || !run->stopped || !run->width || !run->height || !run->fps_num || !run->fps_den) return -1;
    *w = run->width; *h = run->height; *n = run->fps_num; *d = run->fps_den;
    return 0;
}

int sxc_linux_camera_clock_pair(SxcCameraRun *run, guint64 *gst, guint64 *mono) {
    if (!run || !run->clock) return -1;
    struct timespec before, after;
    if (clock_gettime(CLOCK_MONOTONIC, &before) != 0) return -1;
    *gst = gst_clock_get_time(run->clock);
    if (clock_gettime(CLOCK_MONOTONIC, &after) != 0) return -1;
    guint64 lo = (guint64)before.tv_sec * 1000000000ULL + (guint64)before.tv_nsec;
    guint64 hi = (guint64)after.tv_sec * 1000000000ULL + (guint64)after.tv_nsec;
    if (hi < lo || hi - lo > 1000000ULL) return -1;
    *mono = lo + (hi - lo) / 2;
    return 0;
}

/* Internal ABI regression: exercise the actual pre-encoder timestamp probe
   without opening a V4L2 device or creating a media artifact. */
static GstPadProbeReturn test_sample(SxcCameraRun *run, GstClockTime pts) {
    GstBuffer *buffer = gst_buffer_new();
    GST_BUFFER_PTS(buffer) = pts;
    GST_BUFFER_DURATION(buffer) = GST_SECOND / 30;
    GstPadProbeInfo info = { 0 };
    info.type = GST_PAD_PROBE_TYPE_BUFFER;
    info.data = buffer;
    GstPadProbeReturn result = observe(NULL, &info, run);
    gst_buffer_unref(buffer);
    return result;
}

int sxc_linux_camera_test_timestamp_contract(void) {
    if (!gst_init_check(NULL, NULL, NULL)) return -1;
    SxcCameraRun run = { 0 };
    g_mutex_init(&run.mutex);
    run.intervals = g_array_new(FALSE, FALSE, sizeof(SxcInterval));
    run.pipeline = gst_pipeline_new("camera-timestamp-test");
    if (!run.pipeline) { g_array_free(run.intervals, TRUE); g_mutex_clear(&run.mutex); return -1; }
    gst_element_set_base_time(run.pipeline, 1000 * GST_SECOND);
    gst_segment_init(&run.segment, GST_FORMAT_TIME);
    run.has_segment = TRUE;
    run.opened = TRUE;
    run.width = 640; run.height = 480; run.fps_num = 30; run.fps_den = 1;
    int result = 0;
    if (test_sample(&run, 0) != GST_PAD_PROBE_OK || run.intervals->len != 1 || run.bad)
        result = 1;
    if (test_sample(&run, 0) != GST_PAD_PROBE_DROP || run.intervals->len != 1 || run.bad)
        result = 2;
    if (test_sample(&run, GST_SECOND / 30) != GST_PAD_PROBE_OK ||
        run.intervals->len != 2 || run.bad) result = 3;
    if (!fresh_last_interval(&run, 1000 * GST_SECOND + 2 * GST_SECOND, 2000) ||
        fresh_last_interval(&run, 1000 * GST_SECOND + 15 * GST_SECOND, 2000) ||
        fresh_last_interval(&run, GST_CLOCK_TIME_NONE, 2000)) result = 4;
    /* EOS may deliver a valid frame later than the Stop-request clock. The
       already fresh snapshot remains valid, while the later frame still has
       to pass the native timestamp observer. */
    gboolean fresh_at_stop = fresh_last_interval(&run, 1000 * GST_SECOND + GST_SECOND, 2000);
    if (test_sample(&run, 2 * GST_SECOND) != GST_PAD_PROBE_OK ||
        run.intervals->len != 3 || run.bad || !fresh_at_stop ||
        fresh_last_interval(&run, 1000 * GST_SECOND + GST_SECOND, 2000)) result = 7;
    gboolean stale_at_stop = fresh_last_interval(&run, 1000 * GST_SECOND + 15 * GST_SECOND, 2000);
    if (test_sample(&run, 14 * GST_SECOND) != GST_PAD_PROBE_OK ||
        run.intervals->len != 4 || run.bad || stale_at_stop ||
        !fresh_last_interval(&run, 1000 * GST_SECOND + 15 * GST_SECOND, 2000) ||
        fresh_last_interval(&run, GST_CLOCK_TIME_NONE, 2000)) result = 8;
    if (test_sample(&run, GST_SECOND / 60) != GST_PAD_PROBE_DROP || !run.bad ||
        run.intervals->len != 4) result = 5;
    run.bad = FALSE;
    if (test_sample(&run, GST_CLOCK_TIME_NONE) != GST_PAD_PROBE_DROP || !run.bad ||
        run.intervals->len != 4) result = 6;
    gst_object_unref(run.pipeline);
    g_array_free(run.intervals, TRUE);
    g_mutex_clear(&run.mutex);
    return result;
}

/* Internal ABI test seam: a real GStreamer source that emits EOS, no buffers. */
SxcCameraRun *sxc_linux_camera_test_empty(void) {
    if (!gst_init_check(NULL, NULL, NULL)) return NULL;
    SxcCameraRun *run = g_new0(SxcCameraRun, 1);
    g_mutex_init(&run->mutex);
    run->intervals = g_array_new(FALSE, FALSE, sizeof(SxcInterval));
    run->pipeline = gst_pipeline_new("empty-camera-test");
    run->source = gst_element_factory_make("fakesrc", NULL);
    GstElement *sink = gst_element_factory_make("fakesink", NULL);
    if (!run->pipeline || !run->source || !sink) {
        if (sink) gst_object_unref(sink);
        if (run->source) { gst_object_unref(run->source); run->source = NULL; }
        release_run(run);
        return NULL;
    }
    g_object_set(run->source, "num-buffers", 0, NULL);
    gst_bin_add_many(GST_BIN(run->pipeline), run->source, sink, NULL);
    if (!gst_element_link(run->source, sink) ||
        gst_element_set_state(run->pipeline, GST_STATE_PLAYING) == GST_STATE_CHANGE_FAILURE) {
        if (gst_element_set_state(run->pipeline, GST_STATE_NULL) == GST_STATE_CHANGE_FAILURE) return run;
        release_run(run);
        return NULL;
    }
    return run;
}
