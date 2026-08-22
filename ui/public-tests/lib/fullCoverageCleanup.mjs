export async function runWithCleanup(action, cleanup, label = 'coverage action') {
  let value
  let actionError = null
  try {
    value = await action()
  } catch (error) {
    actionError = error
  }

  let cleanupError = null
  try {
    await cleanup()
  } catch (error) {
    cleanupError = error
  }

  if (actionError && cleanupError) {
    throw new AggregateError([actionError, cleanupError], `${label} and cleanup both failed`)
  }
  if (actionError) throw actionError
  if (cleanupError) throw cleanupError
  return value
}
