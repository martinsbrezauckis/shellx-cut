// thirdPartyNotices.ts — load the bundled THIRD-PARTY-NOTICES.txt for Settings > About.
//
// Role: the installers ship a generated list of the libraries compiled into the app (with their copyright lines and
// licence texts) next to NOTICE; the engine serves that one file as plain text at /api/third-party-notices. This
// loader returns the text or an honest, specific failure — it never invents text or swallows a refusal.
// Callers: panels/Environment/ThirdPartyNotices.tsx. Pure: the fetch function is injected for tests.

import { API_BASE } from './clientUrls'

export const THIRD_PARTY_NOTICES_URL = `${API_BASE}/api/third-party-notices`

export type ThirdPartyNoticesResult = { ok: true; text: string } | { ok: false; message: string }

export async function loadThirdPartyNotices(
  fetcher: (url: string) => Promise<{ ok: boolean; status: number; text: () => Promise<string> }> = (url) => fetch(url),
): Promise<ThirdPartyNoticesResult> {
  let response
  try {
    response = await fetcher(THIRD_PARTY_NOTICES_URL)
  } catch (error) {
    return {
      ok: false,
      message: `The engine did not answer the notices request (${error instanceof Error ? error.message : 'network error'}). The same text is in THIRD-PARTY-NOTICES.txt next to the installed app and in the public repository.`,
    }
  }
  const body = await response.text().catch(() => '')
  if (!response.ok) {
    return {
      ok: false,
      message: body.trim() || `The engine answered ${response.status} for the third-party notices. The same text is in THIRD-PARTY-NOTICES.txt next to the installed app and in the public repository.`,
    }
  }
  if (!body.trim()) return { ok: false, message: 'The third-party notices file is empty. Reinstall ShellX Cut, or read THIRD-PARTY-NOTICES.txt in the public repository.' }
  return { ok: true, text: body }
}
