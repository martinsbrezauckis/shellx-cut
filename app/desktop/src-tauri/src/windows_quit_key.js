(() => {
  if (window.__shellxCutQuitKeyBound) return
  window.__shellxCutQuitKeyBound = true
  window.addEventListener('keydown', (event) => {
    if (!event.ctrlKey || event.shiftKey || event.altKey || event.metaKey
      || event.key.toLowerCase() !== 'q') return
    event.preventDefault()
    event.stopImmediatePropagation()
    if (!event.repeat) {
      window.__TAURI__.core.invoke('request_window_close')
        .catch((error) => console.error('Could not request Quit', error))
    }
  }, true)
})()
