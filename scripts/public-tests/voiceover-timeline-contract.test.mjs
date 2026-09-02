import assert from 'node:assert/strict'
import test from 'node:test'
import { readFile } from 'node:fs/promises'

const root = new URL('../../', import.meta.url)
const read = (path) => readFile(new URL(path, root), 'utf8')

test('voiceover coordinator binds the full lifecycle to a volatile owner claim and preserves response-loss retry semantics', async () => {
  const [coordinator, owner, ownership, lifecycle, session, ownerTests, retryTests, ownershipTests, durableTests, placementTests, requestControl, receipt, schema] = await Promise.all([
    read('app/server/src/voiceover_timeline_coordinator.rs'),
    read('app/server/src/voiceover_timeline_owner.rs'),
    read('app/server/src/voiceover_timeline_owner/ownership.rs'),
    read('app/server/src/voiceover_timeline_owner/lifecycle.rs'),
    read('app/server/src/voiceover_timeline_owner/session.rs'),
    read('app/server/src/voiceover_timeline_owner_tests.rs'),
    read('app/server/src/voiceover_timeline_owner_retry_tests.rs'),
    read('app/server/src/voiceover_timeline_owner_ownership_tests.rs'),
    read('app/server/src/voiceover_timeline_owner_durable_tests.rs'),
    read('app/server/src/voiceover_timeline_owner/placement_tests.rs'),
    read('app/server/src/request_control.rs'),
    read('app/server/src/request_control/receipt.rs'),
    read('schema/verbs.json'),
  ])
  for (const marker of [
    'request_control', 'retry_status(&request, &actor', 'stale_request',
    'voiceover_tick_gate',
    'owner.authorize(&actor, &claim)?', 'owner.stop()?', 'owner.cancel()?', 'owner.observe_program_playhead(args.playhead_ms)?',
    'owner.verify_preview_observation(', 'request_fingerprint', 'bridge_epoch',
    'owner.place_materialization(store)?', 'owner.release_after_placement()?',
    'finalize_terminal_without_ops', 'A concurrently accepted Stop/Cancel wins the terminal race',
  ]) assert.match(coordinator, new RegExp(marker.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')))
  for (const marker of ['getrandom::fill', 'opaque_token(32)', 'constant_time_eq', 'authorize_retry', 'owner_refused']) assert.match(ownership, new RegExp(marker.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')))
  assert.doesNotMatch(ownership, /Serialize|Deserialize/)
  assert.match(lifecycle, /validate_preview_observation/)
  assert.match(session, /voiceover Out bridge claim is stale or mismatched/)
  assert.match(requestControl, /fingerprint_args\(name, &args\)/)
  assert.match(requestControl, /voiceover_reattach_session_keeps_the_original_start_fingerprint/)
  assert.match(requestControl, /voiceover_start_defers_preflight_revision_for_active_owner_reattach/)
  assert.match(requestControl, /defers_revision_guard_to_coordinator\(verb\)/)
  assert.match(receipt, /replay_without_ops/)
  assert.ok(requestControl.indexOf('replay_without_ops') < requestControl.indexOf('defers_revision_guard_to_coordinator(verb)'), 'terminal durable receipts replay before voiceover defers its live revision guard')
  assert.match(receipt, /write_without_ops/)
  assert.match(ownership, /release_after_placement/)
  assert.match(coordinator, /let status = owner\s*\.accept_native\([\s\S]{0,500}\)\s*\.map_err\(start_retry_rejected\)\?;[\s\S]{0,400}let claim = owner\.owner_claim\(\)\?;/)
  assert.match(ownerTests, /in_out_stops_only_from_observed_program_playhead_after_countdown/)
  assert.match(ownerTests, /retry_status_returns_only_for_the_exact_live_admission/)
  assert.match(ownerTests, /voiceover_timeline_owner_retry_tests/)
  assert.match(retryTests, /exact_active_retry_survives_a_later_project_revision_and_keeps_stop_cancel_authority/)
  assert.match(retryTests, /new_b_is_refused_while_active_a_owns_the_capture/)
  assert.ok(coordinator.indexOf('owner.retry_status(&request, &actor') < coordinator.indexOf('if current_revision != request.expected_revision'), 'the exact active retry probes before revision comparison')
  assert.match(coordinator, /voiceover_start_retry_rejected/)
  assert.match(ownershipTests, /owner_capability_refuses_foreign_actor_and_stale_preview_out_claim/)
  assert.match(durableTests, /cancel_is_first_wins_and_never_executes_a_later_stop_or_out/)
  assert.match(placementTests, /placement_retries_exactly_and_one_undo_removes_its_asset_and_clip/)
  assert.match(placementTests, /placement_preserves_original_controlled_actor_for_lost_response_replay/)
  assert.match(placementTests, /store\.undo\(Actor::system\(\)\)/)
  const names = JSON.parse(schema).verbs.map((verb) => verb.name)
  for (const name of ['voiceover.start', 'voiceover.tick', 'voiceover.stop', 'voiceover.cancel', 'voiceover.observe_playhead']) assert.ok(names.includes(name), `${name} is public contract`)
})

test('Preview only acknowledges an exact correlated voiceover playback request after seek and playback', async () => {
  const [controller, playbackCommand, preview, playback, events, placement, runtime] = await Promise.all([
    read('ui/src/app/useUiCommandController.ts'),
    read('ui/src/app/voiceoverPlaybackCommand.ts'),
    read('ui/src/panels/Preview/index.tsx'),
    read('ui/src/panels/Preview/useVoiceoverPlayback.ts'),
    read('ui/src/lib/events.ts'),
    read('app/server/src/voiceover_timeline_owner/placement.rs'),
    read('ui/src/app/voiceoverOwnerRuntime.ts'),
  ])
  for (const marker of ['preview.voiceover.playback', 'voiceover_playback']) assert.match(controller, new RegExp(marker.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')))
  for (const marker of ['request_fingerprint', 'bridge_epoch', 'accepted_revision', 'activeVoiceoverOwner', 'this Preview tab does not own the active voiceover take', 'owner_capability']) assert.match(playbackCommand, new RegExp(marker.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')))
  assert.match(playbackCommand, /data-cut-panel="preview"\]\[data-cut-playing="true"\]/)
  assert.match(preview, /useVoiceoverPlayback/)
  assert.match(playback, /cut:voiceover-playback/)
  assert.match(playback, /voiceover\.observe_playhead/)
  assert.match(playback, /request_fingerprint: voiceoverPlayback\.requestFingerprint/)
  assert.match(playback, /bridge_epoch: voiceoverPlayback\.bridgeEpoch/)
  assert.match(playback, /response\.ok && response\.result/)
  assert.match(events, /preview\.voiceover\.playback/)
  assert.match(placement, /Preview acknowledgement is stale or mismatched/)
  assert.match(runtime, /export function persistVoiceoverReattach/)
  assert.match(runtime, /persistReattach\(identity\)/)
  assert.doesNotMatch(runtime, /owner_session_id/)
  assert.doesNotMatch(runtime, /persistReattach\([^)]*capability/)
})
