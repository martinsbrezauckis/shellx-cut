import { API_BASE } from './clientUrls'

export interface AgentDiscovery {
  schema: 'shellx-cut/agent-docs/2'
  product: string
  version: string
  runtime: {
    addr?: string | null
    executable: string
    mcp_proxy: {
      command: string
      args: ['mcp']
      mode: 'proxy'
      authority: string
    }
    standalone: {
      command: string
      args: ['mcp', '--standalone']
      advanced_only: true
      warning: string
    }
  }
  mcp_client_config: {
    mcpServers: {
      'shellx-cut': {
        command: string
        args: ['mcp']
      }
    }
  }
  self_test: {
    verb: 'system.mcp_test'
    read_only: true
    checks: string[]
  }
}

export async function fetchAgentDiscovery(signal?: AbortSignal): Promise<AgentDiscovery> {
  const response = await fetch(`${API_BASE}/api/agent`, { signal })
  if (!response.ok) throw new Error(`Agent discovery returned HTTP ${response.status}`)
  const value = await response.json() as AgentDiscovery
  if (value.schema !== 'shellx-cut/agent-docs/2') {
    throw new Error(`Unsupported agent discovery schema: ${String(value.schema)}`)
  }
  return value
}

export function mcpConfigText(discovery: AgentDiscovery): string {
  return JSON.stringify(discovery.mcp_client_config, null, 2)
}
