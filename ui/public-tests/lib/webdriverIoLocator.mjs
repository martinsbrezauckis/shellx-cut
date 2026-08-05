import { writeFile } from 'node:fs/promises'

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function timeoutFrom(options, fallback) {
  const value = Number(options?.timeout)
  return Number.isFinite(value) && value >= 0 ? value : fallback
}

function keySequence(value) {
  const parts = String(value).split('+')
  if (parts.length === 1) return [parts[0]]
  const aliases = {
    Control: 'Control',
    Ctrl: 'Control',
    Meta: 'Meta',
    Command: 'Meta',
    Shift: 'Shift',
    Alt: 'Alt',
  }
  return [...parts.slice(0, -1).map((part) => aliases[part] || part), parts.at(-1), 'NULL']
}

const WEBDRIVER_MODIFIER_KEYS = {
  Shift: '\uE008',
  Control: '\uE009',
  Alt: '\uE00A',
  Meta: '\uE03D',
}

export function createWebdriverIoLocatorFactory({
  browser,
  installInstrumentation,
  drainEvents,
  recordSuccessfulAction,
  defaultTimeout,
}) {
  async function evaluateChain(chain, callback, arg, all = false) {
    await installInstrumentation()
    // Construct the trusted test callback on the Node side. Building it inside
    // the app would require `unsafe-eval`, which Cut intentionally denies.
    const executeInPage = Function('steps', 'callbackArg', 'useAll', `
      let current = [document]
      for (const step of steps) {
        let found = current.flatMap((root) => [...root.querySelectorAll(step.selector)])
        if (step.hasText) {
          const normalize = (value) => String(value ?? '').replace(/\\s+/g, ' ').trim()
          found = found.filter((node) => {
            const text = normalize(node.textContent)
            if (step.hasText.kind === 'regexp') {
              return new RegExp(step.hasText.source, step.hasText.flags).test(text)
            }
            return text.toLocaleLowerCase().includes(normalize(step.hasText.value).toLocaleLowerCase())
          })
        }
        if (step.index === 'first') current = found.slice(0, 1)
        else if (step.index === 'last') current = found.slice(-1)
        else if (Number.isInteger(step.index)) current = found.slice(step.index, step.index + 1)
        else current = found
      }
      const evaluator = (${callback.toString()})
      return evaluator(useAll ? current : current[0], callbackArg)
    `)
    return browser.execute(executeInPage, chain, arg, all)
  }

  return class WebdriverIoLocator {
    constructor(chain) {
      this.chain = chain
    }

    locator(selector) {
      return new WebdriverIoLocator([...this.chain, { selector }])
    }

    filter(options = {}) {
      if (!Object.hasOwn(options, 'hasText')) {
        throw new Error('WebdriverIO locator.filter currently requires hasText')
      }
      const chain = [...this.chain]
      const hasText = options.hasText instanceof RegExp
        ? {
            kind: 'regexp',
            source: options.hasText.source,
            flags: options.hasText.flags,
          }
        : {
            kind: 'string',
            value: String(options.hasText),
          }
      chain[chain.length - 1] = { ...chain.at(-1), hasText }
      return new WebdriverIoLocator(chain)
    }

    first() {
      const chain = [...this.chain]
      chain[chain.length - 1] = { ...chain.at(-1), index: 'first' }
      return new WebdriverIoLocator(chain)
    }

    last() {
      const chain = [...this.chain]
      chain[chain.length - 1] = { ...chain.at(-1), index: 'last' }
      return new WebdriverIoLocator(chain)
    }

    nth(index) {
      const chain = [...this.chain]
      chain[chain.length - 1] = { ...chain.at(-1), index: Number(index) }
      return new WebdriverIoLocator(chain)
    }

    async elements() {
      return evaluateChain(this.chain, (nodes) => nodes.map((node, index) => ({
        index,
        tagName: node.tagName,
      })), undefined, true)
    }

    async element() {
      return (await this.count()) > 0 ? { chain: this.chain } : null
    }

    async count() {
      return evaluateChain(this.chain, (nodes) => nodes.length, undefined, true)
    }

    async click(options = {}) {
      await this.requireElement()
      const platform = String(browser.capabilities?.platformName || '').toLowerCase()
      const modifiers = (options.modifiers || []).map((modifier) => (
        modifier === 'ControlOrMeta'
          ? (platform.includes('mac') ? 'Meta' : 'Control')
          : modifier
      ))
      const mouseButton = options.button === 'right'
        ? 2
        : options.button === 'middle'
          ? 1
          : 0
      const successfulActionCandidates = mouseButton === 0 && recordSuccessfulAction
        ? await evaluateChain(this.chain, (node) => {
            const out = []
            const direct = node.getAttribute('data-cut-action')
            if (direct) out.push(direct)
            for (const attribute of node.attributes) {
              if (!attribute.name.startsWith('data-cut-') || attribute.name === 'data-cut-action') continue
              out.push(attribute.name.slice('data-cut-'.length))
            }
            return [...new Set(out.filter(Boolean))]
          })
        : []
      if (options.force) {
        await evaluateChain(this.chain, (node, interaction) => {
          const rect = node.getBoundingClientRect()
          const position = interaction.position
          const pointer = {
            clientX: rect.left + (position?.x ?? (rect.width / 2)),
            clientY: rect.top + (position?.y ?? (rect.height / 2)),
          }
          const modifierState = {
            altKey: interaction.modifiers.includes('Alt'),
            ctrlKey: interaction.modifiers.includes('Control'),
            metaKey: interaction.modifiers.includes('Meta'),
            shiftKey: interaction.modifiers.includes('Shift'),
          }
          if (interaction.button === 2) {
            node.dispatchEvent(new MouseEvent('contextmenu', {
              bubbles: true,
              cancelable: true,
              button: 2,
              buttons: 2,
              ...pointer,
              ...modifierState,
            }))
          } else if (interaction.modifiers.length > 0) {
            node.dispatchEvent(new MouseEvent('click', {
              bubbles: true,
              cancelable: true,
              button: interaction.button,
              buttons: 1 << interaction.button,
              ...pointer,
              ...modifierState,
            }))
          } else {
            node.click()
          }
        }, { button: mouseButton, modifiers, position: options.position || null })
      } else {
        await this.scrollIntoViewIfNeeded()
        const box = await this.boundingBox()
        if (!box || box.width <= 0 || box.height <= 0) {
          throw new Error(`locator is not clickable: ${this.selector()}`)
        }
        const pointer = {
          x: Math.round(box.x + (options.position?.x ?? (box.width / 2))),
          y: Math.round(box.y + (options.position?.y ?? (box.height / 2))),
        }
        if (modifiers.length > 0) {
          const modifierValues = modifiers.map((modifier) => WEBDRIVER_MODIFIER_KEYS[modifier] || modifier)
          const pause = () => ({ type: 'pause', duration: 0 })
          await browser.performActions([
            {
              type: 'key',
              id: 'shellx-cut-modifiers',
              actions: [
                ...modifierValues.map((value) => ({ type: 'keyDown', value })),
                pause(),
                pause(),
                pause(),
                ...[...modifierValues].reverse().map((value) => ({ type: 'keyUp', value })),
              ],
            },
            {
              type: 'pointer',
              id: 'shellx-cut-pointer',
              parameters: { pointerType: 'mouse' },
              actions: [
                ...modifierValues.map(pause),
                {
                  type: 'pointerMove',
                  ...pointer,
                  duration: 100,
                  origin: 'viewport',
                },
                { type: 'pointerDown', button: mouseButton },
                { type: 'pointerUp', button: mouseButton },
                ...modifierValues.map(pause),
              ],
            },
          ])
          await browser.releaseActions()
        } else {
          await browser.action('pointer', {
            parameters: { pointerType: 'mouse' },
          })
            .move({
              ...pointer,
            })
            .down({ button: mouseButton })
            .up({ button: mouseButton })
            .perform()
        }
      }
      // Native pickers can synchronously block the WebView click command, and
      // mousedown-driven scrims can unmount before `click` reaches document.
      // Record a fallback only after WebDriver reports a successful pointer
      // interaction and only for candidates the DOM recorder did not queue.
      if (successfulActionCandidates.length) {
        await recordSuccessfulAction(successfulActionCandidates)
      }
      await drainEvents()
    }

    async fill(value) {
      await this.requireElement()
      await evaluateChain(this.chain, (node, nextValue) => {
        node.focus()
        const prototype = node instanceof HTMLTextAreaElement
          ? HTMLTextAreaElement.prototype
          : node instanceof HTMLSelectElement
            ? HTMLSelectElement.prototype
            : HTMLInputElement.prototype
        const setter = Object.getOwnPropertyDescriptor(prototype, 'value')?.set
        if (setter) setter.call(node, nextValue)
        else node.value = nextValue
        node.dispatchEvent(new InputEvent('input', {
          bubbles: true,
          inputType: 'insertText',
          data: nextValue,
        }))
        node.dispatchEvent(new Event('change', { bubbles: true }))
      }, String(value))
      await drainEvents()
    }

    async focus() {
      await this.requireElement()
      await evaluateChain(this.chain, (node) => node.focus())
      await drainEvents()
    }

    async hover() {
      await this.requireElement()
      await this.scrollIntoViewIfNeeded()
      const box = await this.boundingBox()
      if (!box || box.width <= 0 || box.height <= 0) {
        throw new Error(`locator cannot be hovered: ${this.selector()}`)
      }
      await browser.action('pointer', {
        parameters: { pointerType: 'mouse' },
      })
        .move({
          x: Math.round(box.x + (box.width / 2)),
          y: Math.round(box.y + (box.height / 2)),
        })
        .perform()
      await drainEvents()
    }

    async press(value) {
      await this.focus()
      await browser.keys(keySequence(value))
      await drainEvents()
    }

    async selectOption(option) {
      await this.requireElement()
      await evaluateChain(this.chain, (node, selection) => {
        const options = [...node.options]
        const match = selection.kind === 'value'
          ? options.find((candidate) => candidate.value === selection.value)
          : selection.kind === 'label'
            ? options.find((candidate) => candidate.text === selection.value)
            : options[selection.value]
        if (!match) throw new Error(`select option did not resolve: ${selection.value}`)
        node.value = match.value
        node.dispatchEvent(new Event('input', { bubbles: true }))
        node.dispatchEvent(new Event('change', { bubbles: true }))
      }, typeof option === 'string'
        ? { kind: 'value', value: option }
        : option && typeof option.label === 'string'
          ? { kind: 'label', value: option.label }
          : option && Number.isInteger(option.index)
            ? { kind: 'index', value: option.index }
            : { kind: 'unsupported', value: JSON.stringify(option) })
      await drainEvents()
    }

    async getAttribute(name) {
      if (!(await this.count())) return null
      return evaluateChain(this.chain, (node, attribute) => node.getAttribute(attribute), name)
    }

    async textContent() {
      if (!(await this.count())) return null
      return evaluateChain(this.chain, (node) => node.textContent)
    }

    async inputValue() {
      if (!(await this.count())) return ''
      return evaluateChain(this.chain, (node) => String(node.value ?? ''))
    }

    async isVisible() {
      if (!(await this.count())) return false
      return evaluateChain(this.chain, (node) => {
        const style = getComputedStyle(node)
        const rect = node.getBoundingClientRect()
        return style.display !== 'none' &&
          style.visibility !== 'hidden' &&
          Number(style.opacity || 1) > 0 &&
          rect.width > 0 &&
          rect.height > 0
      })
    }

    async isDisabled() {
      if (!(await this.count())) return true
      return evaluateChain(
        this.chain,
        (node) => Boolean(node.disabled || node.getAttribute('aria-disabled') === 'true'),
      )
    }

    async isEnabled() {
      return !(await this.isDisabled())
    }

    async isChecked() {
      if (!(await this.count())) return false
      return evaluateChain(
        this.chain,
        (node) => Boolean(node.checked || node.getAttribute('aria-checked') === 'true'),
      )
    }

    async boundingBox() {
      if (!(await this.count())) return null
      return evaluateChain(this.chain, (node) => {
        const rect = node.getBoundingClientRect()
        return { x: rect.x, y: rect.y, width: rect.width, height: rect.height }
      })
    }

    async evaluate(fn, arg) {
      await this.requireElement()
      const result = await evaluateChain(this.chain, fn, arg)
      await drainEvents()
      return result
    }

    async evaluateAll(fn, arg) {
      const result = await evaluateChain(this.chain, fn, arg, true)
      await drainEvents()
      return result
    }

    async waitFor(options = {}) {
      const state = options.state || 'visible'
      const timeout = timeoutFrom(options, defaultTimeout())
      const deadline = Date.now() + timeout
      while (Date.now() <= deadline) {
        const count = await this.count()
        const visible = count > 0 && await this.first().isVisible().catch(() => false)
        if (
          (state === 'attached' && count > 0) ||
          (state === 'detached' && count === 0) ||
          (state === 'visible' && visible) ||
          (state === 'hidden' && !visible)
        ) return
        await sleep(80)
      }
      throw new Error(`locator did not become ${state}: ${this.selector()}`)
    }

    async scrollIntoViewIfNeeded() {
      await this.requireElement()
      await evaluateChain(
        this.chain,
        (node) => {
          const rect = node.getBoundingClientRect()
          const viewportWidth = Math.max(
            Number(globalThis.innerWidth) || 0,
            Number(document.documentElement?.clientWidth) || 0,
          )
          const viewportHeight = Math.max(
            Number(globalThis.innerHeight) || 0,
            Number(document.documentElement?.clientHeight) || 0,
          )
          const fullyVisible = rect.width > 0
            && rect.height > 0
            && rect.left >= 0
            && rect.top >= 0
            && rect.right <= viewportWidth
            && rect.bottom <= viewportHeight
          const hit = fullyVisible
            ? document.elementFromPoint(
                rect.left + (rect.width / 2),
                rect.top + (rect.height / 2),
              )
            : null
          const centerReachable = !!hit && (hit === node || node.contains(hit))
          if (!fullyVisible || !centerReachable) {
            node.scrollIntoView({ block: 'center', inline: 'center' })
          }
        },
      )
    }

    async screenshot({ path }) {
      await this.requireElement()
      const data = await browser.takeScreenshot()
      await writeFile(path, Buffer.from(data, 'base64'))
    }

    async check() {
      if (!(await this.isChecked())) await this.click()
    }

    async dispatchEvent(type, init = {}) {
      await this.requireElement()
      await evaluateChain(this.chain, (node, detail) => {
        const { eventType, eventInit } = detail
        const EventClass = eventType.startsWith('mouse') || eventType === 'contextmenu'
          ? MouseEvent
          : Event
        node.dispatchEvent(new EventClass(eventType, { bubbles: true, cancelable: true, ...eventInit }))
      }, { eventType: type, eventInit: init })
      await drainEvents()
    }

    selector() {
      return this.chain.at(-1)?.selector || '<unknown>'
    }

    async requireElement() {
      const element = await this.element()
      if (!element) throw new Error(`locator did not resolve: ${this.selector()}`)
      return element
    }
  }
}

export { keySequence }
