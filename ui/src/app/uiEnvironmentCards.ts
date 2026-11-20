// The Settings tool entries as rendered (ui.state overlays.settings_cards), so
// an agent sees what the person sees: each visible card's id, its status and
// whether its install action is offered (e.g. the Microsoft Visual C++ runtime
// entry before and after it is installed). The engine's truth stays
// system.doctor; this is the screen's.

export interface EnvironmentCardState {
  id: string
  status: string
  /** The card offers its download / install button right now. */
  install_offered: boolean
}

type CardDocument = Partial<Pick<Document, 'querySelectorAll'>>

export function readEnvironmentCards(documentRef: CardDocument): EnvironmentCardState[] {
  if (!documentRef.querySelectorAll) return []
  return Array.from(documentRef.querySelectorAll<HTMLElement>('[data-cut-env-card]')).map((card) => {
    const id = card.dataset.cutEnvCard ?? ''
    return {
      id,
      status: card.dataset.cutEnvStatus ?? 'unknown',
      install_offered: card.querySelector(
        '[data-cut-env-download],[data-cut-env-setup-perception],[data-cut-env-setup-matte]',
      ) !== null,
    }
  })
}
