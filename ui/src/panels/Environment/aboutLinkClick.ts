import { isTauri, openAboutLink, type AboutLinkDestination } from '../../lib/tauri'

/** Keep normal anchor navigation in a browser; route desktop clicks through the shell. */
export async function handleAboutLinkClick(
  destination: AboutLinkDestination,
  version: string | undefined,
  event: { preventDefault: () => void },
  showError: (message: string | null) => void,
  desktop = isTauri(),
  opener = openAboutLink,
): Promise<void> {
  if (!desktop) return
  event.preventDefault()
  showError(null)
  const result = await opener(destination, version)
  if (!result.ok) showError(result.message)
}
