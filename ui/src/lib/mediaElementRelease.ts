import { useCallback, useEffect, useRef, type RefObject } from 'react'

/** Stop network/decoder pipelines when a media element unmounts or switches elements. */
export function releaseMediaElement(element: HTMLMediaElement | null | undefined): void {
  if (!element) return
  try {
    element.pause()
  } catch {}
  element.removeAttribute('src')
  try {
    element.load()
  } catch {}
}

/** Callback ref that releases the attached media element on replacement or unmount. */
export function useReleasedMediaRef<T extends HTMLMediaElement>(target: RefObject<T | null>) {
  const attached = useRef<T | null>(null)
  useEffect(() => () => {
    releaseMediaElement(attached.current)
    attached.current = null
    target.current = null
  }, [target])
  return useCallback((element: T | null) => {
    if (attached.current && attached.current !== element) releaseMediaElement(attached.current)
    attached.current = element
    target.current = element
  }, [target])
}
