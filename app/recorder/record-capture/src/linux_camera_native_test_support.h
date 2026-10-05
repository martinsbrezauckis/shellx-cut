/* Native test seams share the camera owner translation unit and its static helpers. */
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
