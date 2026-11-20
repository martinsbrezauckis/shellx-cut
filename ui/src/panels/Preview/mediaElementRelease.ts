import { useCallback, type RefObject } from 'react'

/** Stop a media element and drop its source so the webview tears down its
 * decoder and network stream now. Without this a removed element keeps its
 * media pipeline until it is garbage collected, and repeated mounts (the base
 * video disappears in every base-track gap) pile up live pipelines. */
export function releaseMediaElement(element: HTMLMediaElement): void {
  element.pause()
  element.removeAttribute('src')
  element.load()
}

/** A callback ref that fills `target` and releases the element on unmount. */
export function useReleasedMediaRef<T extends HTMLMediaElement>(target: RefObject<T | null>) {
  return useCallback((element: T | null) => {
    target.current = element
    if (!element) return undefined
    return () => {
      if (target.current === element) target.current = null
      releaseMediaElement(element)
    }
  }, [target])
}
