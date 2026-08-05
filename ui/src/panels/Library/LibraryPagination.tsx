export interface LibraryPaginationProps {
  offset: number
  limit: number
  total: number
  pageNumber: number
  pageCount: number
  hasNext: boolean
  loading: boolean
  onPrevious: () => void
  onNext: () => void
}

export function LibraryPagination({
  offset,
  limit,
  total,
  pageNumber,
  pageCount,
  hasNext,
  loading,
  onPrevious,
  onNext,
}: LibraryPaginationProps) {
  const first = total === 0 ? 0 : offset + 1
  const last = Math.min(total, offset + limit)
  return (
    <nav className="lb-pagination" data-cut-library-pagination aria-label="Library pages">
      <button
        type="button"
        className="lb-page-btn"
        data-cut-library-page-prev
        disabled={loading || offset === 0}
        onClick={onPrevious}
      >
        Previous
      </button>
      <span data-cut-library-page-status aria-live="polite">
        {first}–{last} of {total} · Page {pageNumber} of {pageCount}
      </span>
      <button
        type="button"
        className="lb-page-btn"
        data-cut-library-page-next
        disabled={loading || !hasNext}
        onClick={onNext}
      >
        Next
      </button>
    </nav>
  )
}
