// Shell fragment shared by the macOS and Linux native full-action runners.
// It replaces only nondeterministic external CLI/model seams; the installed
// product, UI controls, verbs, project mutation, and result checks remain real.
export const FULL_AGENT_FIXTURE_SHELL = String.raw`
if [ "$FCV_AGENT_FIXTURES_VALUE" = "1" ]; then
  export PATH="$REMOTE_DIR_RESOLVED/scripts/release/fixtures:$PATH"
  export CUTD_DRAFT_ADAPTER="$REMOTE_DIR_RESOLVED/scripts/release/fixtures/comment-draft-adapter.py"
  export CUTD_JUDGE_ADAPTER="$REMOTE_DIR_RESOLVED/scripts/release/fixtures/judge-adapter.py"
  export CUTD_GENERATE_PROMPT_ADAPTER="$REMOTE_DIR_RESOLVED/ui/public-tests/fixtures/generate-prompt-adapter.py"
  export CUTD_GENERATE_STORYBOARD_ADAPTER="$REMOTE_DIR_RESOLVED/ui/public-tests/fixtures/generate-storyboard-adapter.py"
  export CUTD_GENERATE_FIXTURE_DELAY_MS=1200
fi
`
