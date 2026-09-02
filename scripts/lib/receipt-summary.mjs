export function summarizeBooleanResults(results) {
  const rows = Array.isArray(results) ? results : [];
  const pass = rows.filter((row) => row?.ok === true).length;
  const fail = rows.filter((row) => row?.ok !== true).length;
  return { total: rows.length, pass, fail };
}

export function withBooleanReceiptSummary(receipt, options = {}) {
  const summary = summarizeBooleanResults(receipt?.results);
  return {
    ...receipt,
    generatedAt: options.generatedAt ?? receipt?.generatedAt ?? new Date().toISOString(),
    version: options.version ?? receipt?.version ?? null,
    ok: summary.fail === 0,
    summary,
  };
}
