// Guards Generate results against a project replacement while an adapter call is in flight.

export interface GenerateRequestToken {
  projectScope: number
  generation: number
}

export class GenerateRequestScopeGuard {
  private projectScope: number
  private generation = 0

  constructor(projectScope: number) {
    this.projectScope = projectScope
  }

  setProjectScope(projectScope: number): void {
    if (projectScope === this.projectScope) return
    this.projectScope = projectScope
    this.generation += 1
  }

  begin(): GenerateRequestToken {
    this.generation += 1
    return { projectScope: this.projectScope, generation: this.generation }
  }

  isCurrent(token: GenerateRequestToken): boolean {
    return token.projectScope === this.projectScope && token.generation === this.generation
  }
}
