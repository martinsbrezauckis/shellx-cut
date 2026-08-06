#!/usr/bin/env node
import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const lineCount = (relative) => {
  const source = readFileSync(resolve(ROOT, relative), "utf8");
  return source.split("\n").length - Number(source.endsWith("\n"));
};

// Feature modules should stay reviewable in one sitting. A module that needs
// more room must be decomposed by responsibility, not added as an exception.
const boundedSources = [
  "app/desktop/src-tauri/examples/verify-updater-signature.rs",
  "app/desktop/src-tauri/src/update_settings.rs",
  "app/server/src/motion_package.rs",
  "app/server/src/motion_package_lineage.rs",
  "app/server/src/motion_package_lineage_io.rs",
  "app/server/src/motion_template_catalog.rs",
  "app/server/src/dispatch/motion_link_projection.rs",
  "app/server/src/motion_tracking/command.rs",
  "app/server/src/motion_tracking/contract.rs",
  "app/server/src/motion_tracking/handlers.rs",
  "app/server/src/motion_tracking/inventory.rs",
  "app/server/src/motion_tracking/link.rs",
  "app/server/src/dispatch/sequence_index.rs",
  "app/server/src/dispatch/sequence_index/rows.rs",
  "app/server/src/jobs/runtime.rs",
  "app/server/src/dispatch/safety.rs",
  "app/server/src/dispatch/project_workspace/library_handlers.rs",
  "app/server/src/dispatch/project_workspace/library_mutations.rs",
  "app/server/src/dispatch/project_workspace/project_paths.rs",
  "app/server/src/schema_validation.rs",
  "app/server/src/ui_bridge.rs",
  "app/server/src/mcp/self_test.rs",
  "ui/src/lib/agentControl.ts",
  "ui/src/lib/motionLinkModel.ts",
  "ui/src/app/uiSurfaceRegistry.ts",
  "ui/src/app/uiControlState.ts",
  "ui/src/app/useUiStatePublisher.ts",
  "ui/src/app/useUiCommandController.ts",
  "ui/src/panels/Inspector/MotionEffectsSection.tsx",
  "ui/src/panels/Inspector/MotionLinkSection.tsx",
  "ui/src/panels/Inspector/MotionTrackingSection.tsx",
  "ui/src/panels/Inspector/AudioInspectorTools.tsx",
  "ui/src/panels/Inspector/VideoColorSection.tsx",
  "ui/src/panels/Inspector/VideoInspectorTools.tsx",
  "ui/src/panels/Inspector/inspectorTaskModel.ts",
  "ui/src/panels/Inspector/inspectorTasks.css",
  "ui/src/panels/Inspector/motion.css",
  "ui/src/panels/SequenceIndex/csv.ts",
  "ui/src/panels/SequenceIndex/index.tsx",
  "ui/src/panels/SequenceIndex/sequenceIndex.css",
  "ui/src/panels/Environment/AgentControl.tsx",
  "ui/src/DropZone.tsx",
  "ui/src/lib/projectBootstrap.ts",
  "scripts/linux-wdio-full-coverage.mjs",
  "scripts/windows-installed-full-coverage.mjs",
  "scripts/ui-action-coverage-audit.mjs",
  "scripts/lib/updater-manifest.mjs",
  "scripts/lib/installed-runtime-evidence.mjs",
  "scripts/lib/installed-walkthrough-receipt.mjs",
  "scripts/lib/native-artifact-integrity.mjs",
  "scripts/lib/windows-installed-walkthrough.mjs",
  "scripts/linux-installed-walkthrough-receipt.mjs",
  "scripts/macos-installed-walkthrough.mjs",
  "scripts/release/generate-updater-manifest.mjs",
  "ui/src/panels/Environment/KeymapEditor.tsx",
  "ui/src/panels/Environment/SettingsCategoryContent.tsx",
  "ui/src/panels/Environment/SettingsOverview.tsx",
  "ui/src/panels/Environment/SettingsShell.tsx",
  "ui/src/panels/Environment/UpdateNetworkSettings.tsx",
  "ui/src/panels/Environment/keymapSettingsModel.ts",
  "ui/src/panels/Environment/settingsModel.ts",
  "ui/src/panels/Library/LibraryActions.tsx",
  "ui/src/panels/Library/LibraryBulkBar.tsx",
  "ui/src/panels/Library/LibraryCard.tsx",
  "ui/src/panels/Library/LibraryCollections.tsx",
  "ui/src/panels/Library/LibraryContextMenus.tsx",
  "ui/src/panels/Library/LibraryDetails.tsx",
  "ui/src/panels/Library/LibraryFilters.tsx",
  "ui/src/panels/Library/LibraryFolders.tsx",
  "ui/src/panels/Library/LibraryPagination.tsx",
  "ui/src/panels/Library/LibraryPoster.tsx",
  "ui/src/panels/Library/LibraryRow.tsx",
  "ui/src/panels/Library/LibraryTags.tsx",
  "ui/src/panels/Library/LibraryWorkspace.tsx",
  "ui/src/panels/Library/libraryPlacement.ts",
  "ui/src/panels/Library/model.ts",
  "ui/src/panels/Library/useLibraryKeyboardNavigation.ts",
  "ui/src/panels/Library/useLibraryQuery.ts",
  "ui/src/panels/Library/useLibraryRelink.ts",
];
for (const file of boundedSources) {
  assert.ok(lineCount(file) <= 350, `${file} exceeds the 350-line feature-module limit`);
}

const boundedTests = [
  "app/server/src/motion_package/tests.rs",
  "app/server/src/dispatch/motion_link_projection/tests.rs",
  "app/server/src/motion_test_fixtures.rs",
  "app/server/src/generate_rich_tests.rs",
  "app/server/src/dispatch/tests/sequence_index.rs",
  "app/server/src/schema_validation/tests.rs",
  "app/server/src/dispatch/tests/ui_command_confirmation.rs",
  "scripts/public-tests/feature-contract.test.mjs",
  "scripts/public-tests/docs-freshness-contract.test.mjs",
  "scripts/public-tests/release-gate-effect-contract.test.mjs",
  "scripts/public-tests/update-check-disclosure-contract.test.mjs",
  "scripts/public-tests/updater-manifest.test.mjs",
  "scripts/public-tests/installed-walkthrough-receipt.test.mjs",
  "scripts/public-tests/human-copy-contract.test.mjs",
  "scripts/public-tests/ui-accessibility-contract.test.mjs",
  "ui/public-tests/motion-effects.test.ts",
  "ui/public-tests/generate-rich-motion.test.ts",
  "ui/public-tests/lib/fullCoverageSettings.mjs",
  "ui/public-tests/lib/fullCoverageSettingsEnvironment.mjs",
  "ui/public-tests/lib/fullCoverageSettingsTasks.mjs",
  "ui/public-tests/lib/fullCoverageSettingsUpdate.mjs",
  "ui/public-tests/lib/fullCoverageLibraryActions.mjs",
  "ui/public-tests/lib/fullCoverageProjectsActions.mjs",
  "ui/public-tests/lib/fullCoverageAssetsActions.mjs",
  "ui/public-tests/lib/fullCoverageAssetsPickerActions.mjs",
  "ui/public-tests/lib/fullCoverageAssetsSetupActions.mjs",
  "ui/public-tests/lib/fullCoverageLayerActions.mjs",
  "ui/public-tests/lib/fullCoverageMaskActions.mjs",
  "ui/public-tests/lib/fullCoverageShapeActions.mjs",
  "ui/public-tests/lib/fullCoverageTitleActions.mjs",
  "ui/public-tests/lib/fullCoverageTimelineToolbarActions.mjs",
  "ui/public-tests/lib/fullCoverageTimelineTrackActions.mjs",
  "ui/public-tests/lib/fullCoverageTimelineDialogActions.mjs",
  "ui/public-tests/lib/fullCoverageNativeOtioActions.mjs",
  "ui/public-tests/lib/fullCoverageRuntimeActionRecorder.mjs",
  "ui/public-tests/lib/fullCoverageUserActionFeedback.mjs",
  "scripts/public-tests/runtime-action-recorder.test.mjs",
  "ui/public-tests/theme-toggle-sync.test.ts",
  "ui/public-tests/stt-model-feedback.test.ts",
  "ui/public-tests/verify-sequence-index.mjs",
  "ui/public-tests/verify-offline-media.mjs",
  "ui/public-tests/ui-control-verify.mjs",
  "ui/public-tests/inspector-discoverability-verify.mjs",
  "app/server/src/motion_bridge/tests/lineage_integrity.rs",
];
for (const file of boundedTests) {
  assert.ok(lineCount(file) <= 600, `${file} exceeds the 600-line test-module limit`);
}

// These modules still benefit from extraction. Their ceilings lock the current
// reduction so later feature work cannot silently grow them again.
const legacyNoGrowth = {
  "app/server/src/main.rs": 368,
  "app/server/src/motion_bridge.rs": 2_732,
  "app/server/src/motion_bridge/tests.rs": 2_715,
  "app/server/src/dispatch/project_workspace.rs": 1_982,
  "app/server/src/dispatch.rs": 1_356,
  "app/core/src/store.rs": 6_402,
  "app/core/src/edit.rs": 4_980,
  "app/core/src/types.rs": 4_155,
  "app/media/src/render.rs": 4_993,
  "app/media/src/title.rs": 3_378,
  "app/server/src/jobs.rs": 1_050,
  "ui/src/lib/clientModel.ts": 615,
  "ui/src/panels/Timeline/index.tsx": 1_765,
  "ui/src/panels/Preview/index.tsx": 1_069,
  "ui/src/panels/Library/index.tsx": 725,
  "ui/src/panels/Library/library.css": 739,
  "ui/src/panels/Environment/EnvCardRow.tsx": 412,
  "ui/src/panels/Environment/environment.css": 767,
  "ui/src/panels/Inspector/index.tsx": 371,
  "ui/src/panels/Inspector/inspector.css": 373,
};
for (const [file, ceiling] of Object.entries(legacyNoGrowth)) {
  assert.ok(lineCount(file) <= ceiling, `${file} grew past its locked ${ceiling}-line ceiling`);
}

console.log(`PASS module-size-gate (${boundedSources.length} bounded modules, ${Object.keys(legacyNoGrowth).length} locked legacy files)`);
