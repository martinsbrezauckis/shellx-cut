import { useEffect, useMemo, useState } from 'react'
import { callVerb, type McpSelfTestResult } from '../../lib/client'
import {
  fetchAgentDiscovery,
  mcpConfigText,
  type AgentDiscovery,
} from '../../lib/agentControl'
import { writeClipboardText } from '../../lib/clipboard'
import type { DoctorReport } from '../../lib/doctor'

type CopyState = 'api' | 'mcp' | 'error' | null
type TestState = 'idle' | 'running' | 'ok' | 'error'

export default function AgentControl({ report }: { report: DoctorReport | null }) {
  const [copyState, setCopyState] = useState<CopyState>(null)
  const [discovery, setDiscovery] = useState<AgentDiscovery | null>(null)
  const [discoveryError, setDiscoveryError] = useState('')
  const [testState, setTestState] = useState<TestState>('idle')
  const [testResult, setTestResult] = useState<McpSelfTestResult | null>(null)
  const [testError, setTestError] = useState('')
  const addr = report?.addr ?? discovery?.runtime.addr ?? ''
  const origin = addr ? (/^https?:\/\//.test(addr) ? addr : `http://${addr}`) : ''
  const restRoute = origin ? `${origin}/api/verb/{name}` : ''
  const loopback = /(?:127\.0\.0\.1|localhost|\[::1\])/.test(origin)
  const config = useMemo(
    () => discovery ? mcpConfigText(discovery) : '',
    [discovery],
  )

  useEffect(() => {
    const controller = new AbortController()
    void fetchAgentDiscovery(controller.signal)
      .then((value) => {
        setDiscovery(value)
        setDiscoveryError('')
      })
      .catch((error: unknown) => {
        if (!controller.signal.aborted) {
          setDiscoveryError(error instanceof Error ? error.message : 'Agent discovery failed')
        }
      })
    return () => controller.abort()
  }, [])

  const copy = async (kind: Exclude<CopyState, 'error' | null>, value: string) => {
    try {
      await writeClipboardText(value)
      setCopyState(kind)
    } catch {
      setCopyState('error')
    }
  }

  const testMcp = async () => {
    setTestState('running')
    setTestResult(null)
    setTestError('')
    try {
      const response = await callVerb('system.mcp_test', {})
      if (!response.ok || !response.result) {
        setTestState('error')
        setTestError(response.error?.message ?? 'The MCP self-test did not complete.')
        return
      }
      setTestResult(response.result)
      setTestState('ok')
    } catch (error) {
      setTestState('error')
      setTestError(error instanceof Error ? error.message : 'Could not reach the local engine.')
    }
  }

  const mcpStatus = testState === 'ok'
    ? 'ok'
    : testState === 'error'
      ? 'missing'
      : discovery
        ? 'degraded'
        : 'unknown'

  return (
    <div className="settings-control-list" data-cut-agent-control>
      <div className="settings-control-row" data-cut-agent-control-api>
        <span className={`settings-status settings-status--${restRoute ? 'ok' : 'unknown'}`} aria-hidden="true" />
        <span className="settings-control-copy">
          <strong>Debug API</strong>
          <span>{restRoute ? (loopback ? 'Connected locally to the running Cut engine.' : 'Warning: engine address is not loopback.') : 'Waiting for the local engine.'}</span>
        </span>
        <div className="settings-control-actions">
          <button type="button" className="env-btn env-btn--ghost" data-cut-agent-control-copy-rest disabled={!restRoute} onClick={() => void copy('api', restRoute)}>
            Copy REST route
          </button>
        </div>
      </div>

      <div className="settings-control-row" data-cut-agent-control-mcp>
        <span className={`settings-status settings-status--${mcpStatus}`} aria-hidden="true" />
        <span className="settings-control-copy">
          <strong>MCP proxy</strong>
          <span>
            {testState === 'ok'
              ? `Connected to this engine with ${testResult?.tools ?? 0} tools.`
              : testState === 'error'
                ? 'Self-test failed; Cut did not claim a working MCP connection.'
                : discovery
                  ? 'Ready to test. Uses the same validated verbs and running project as the Debug API.'
                  : 'Loading the installed MCP command…'}
          </span>
        </span>
        <div className="settings-control-actions">
          <button type="button" className="env-btn env-btn--ghost" data-cut-agent-control-copy-mcp disabled={!config} onClick={() => void copy('mcp', config)}>
            Copy MCP setup
          </button>
          <button
            type="button"
            className="env-btn env-btn--primary"
            data-cut-agent-control-test
            disabled={!discovery || testState === 'running'}
            onClick={() => void testMcp()}
          >
            {testState === 'running' ? 'Testing…' : 'Test MCP'}
          </button>
        </div>
      </div>

      {(copyState || discoveryError || testState === 'ok' || testState === 'error') && (
        <div
          className={`settings-copy-note${copyState === 'error' || discoveryError || testState === 'error' ? ' settings-copy-note--error' : ''}`}
          data-cut-agent-control-test-result
          role="status"
          aria-live="polite"
        >
          {testState === 'error'
            ? testError
            : testState === 'ok' && testResult
              ? `MCP connected · protocol ${testResult.protocol_version} · ${testResult.tools} tools · same engine confirmed.`
              : discoveryError || (copyState === 'error'
                ? 'Could not copy. Open Advanced connection details to select the value.'
                : `${copyState === 'api' ? 'REST route' : 'MCP setup'} copied.`)}
        </div>
      )}

      <div className="settings-authority-note" data-cut-agent-control-client-guide>
        <strong>Connect an MCP client or agent</strong>
        <p>Keep Cut open, copy the MCP setup, then add its <code>mcpServers</code> block to your MCP client configuration. Test MCP verifies this installed proxy returns to the same Cut engine; your external client should then list the <code>shellx-cut</code> tools.</p>
        <p>The Debug API and MCP both dispatch through that one running engine. Proxy mode does not open another project or keep separate edit state.</p>
      </div>

      <details className="settings-advanced" data-cut-agent-control-advanced>
        <summary data-cut-agent-control-advanced-toggle>Advanced connection details</summary>
        <dl>
          <div><dt>REST route</dt><dd><code>{restRoute || 'Not available'}</code></dd></div>
          <div><dt>Installed executable</dt><dd><code>{discovery?.runtime.executable ?? 'Loading…'}</code></dd></div>
          <div><dt>MCP command</dt><dd><code>{discovery ? `${JSON.stringify(discovery.runtime.executable)} mcp` : 'Loading…'}</code></dd></div>
          <div><dt>Default mode</dt><dd>Proxy to the running Cut engine (recommended).</dd></div>
          <div><dt>Standalone</dt><dd>{discovery?.runtime.standalone.warning ?? 'Separate-state mode; advanced testing only.'}</dd></div>
        </dl>
        {config && <pre className="settings-agent-config" data-cut-agent-control-config>{config}</pre>}
      </details>
    </div>
  )
}
