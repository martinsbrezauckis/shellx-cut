/// cpal capture body result. The callback only queues samples; the stream
/// worker owns disk draining and records the first native packet offset.
pub(crate) struct MicStreamEnd {
    pub(crate) microphone_lost: bool,
    pub(crate) samples_written: bool,
    pub(crate) first_packet_offset_ms: Option<u64>,
}
