import { createHash, createPrivateKey, createPublicKey, X509Certificate } from 'node:crypto'
import { createServer, request } from 'node:https'
import { readFileSync } from 'node:fs'

function digest(value) {
  return createHash('sha256').update(value).digest('hex')
}

function readPem(path, label) {
  try {
    const value = readFileSync(path)
    if (!value.length) throw new Error('empty')
    return value
  } catch (error) {
    throw new Error(`${label} is not readable: ${error.message}`)
  }
}

function samePublicKey(left, right) {
  return left.export({ type: 'spki', format: 'der' }).equals(right.export({ type: 'spki', format: 'der' }))
}

/**
 * The rig accepts a single explicit local CA. It neither disables certificate
 * verification nor changes the target operating system trust store; the
 * operator must arrange native trust before launching the installed app.
 */
export function validateLoopbackTls({ certPath, keyPath, caPath, now = Date.now() }) {
  const cert = readPem(certPath, 'TLS certificate')
  const key = readPem(keyPath, 'TLS private key')
  const ca = readPem(caPath, 'TLS CA certificate')
  let leaf
  let issuer
  let privateKey
  try {
    leaf = new X509Certificate(cert)
    issuer = new X509Certificate(ca)
    privateKey = createPrivateKey(key)
  } catch (error) {
    throw new Error(`TLS material is invalid: ${error.message}`)
  }
  if (!leaf.checkIP('127.0.0.1')) throw new Error('TLS certificate must contain 127.0.0.1 as an IP subject alternative name')
  if (Number.isNaN(Date.parse(leaf.validFrom)) || Number.isNaN(Date.parse(leaf.validTo))
      || now < Date.parse(leaf.validFrom) || now > Date.parse(leaf.validTo)) {
    throw new Error('TLS certificate is not currently valid')
  }
  if (!samePublicKey(createPublicKey(privateKey), leaf.publicKey)) throw new Error('TLS private key does not match certificate')
  if (!issuer.ca || !leaf.verify(issuer.publicKey)) throw new Error('TLS certificate is not signed by the supplied CA')
  return {
    cert,
    key,
    ca,
    certificateSha256: digest(cert),
    caSha256: digest(ca),
  }
}

export function requestTrustedBytes(url, ca) {
  return new Promise((resolve, reject) => {
    const req = request(url, {
      ca,
      method: 'GET',
      rejectUnauthorized: true,
    }, (response) => {
      const chunks = []
      response.on('data', (chunk) => chunks.push(chunk))
      response.on('end', () => resolve({ status: response.statusCode || 0, body: Buffer.concat(chunks) }))
    })
    req.once('error', reject)
    req.end()
  })
}

function listen(server, port) {
  return new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen({ host: '127.0.0.1', port }, () => {
      server.off('error', reject)
      const address = server.address()
      if (!address || typeof address === 'string' || address.address !== '127.0.0.1') {
        server.close(() => reject(new Error('staged update server did not bind exclusively to 127.0.0.1')))
        return
      }
      resolve(address)
    })
  })
}

function closeServer(server, sockets) {
  return new Promise((resolve, reject) => {
    for (const socket of sockets) socket.destroy()
    if (typeof server.closeAllConnections === 'function') server.closeAllConnections()
    server.close((error) => error ? reject(error) : resolve())
  })
}

/** Serve immutable, already-hashed buffers and no directory listing or fallback. */
export async function startLoopbackUpdateServer({ preflight, tls }) {
  const feed = new URL(preflight.feedUrl)
  if (feed.protocol !== 'https:' || feed.hostname !== '127.0.0.1' || !feed.port) {
    throw new Error('staged update feed must use an explicit 127.0.0.1 HTTPS URL')
  }
  const manifest = readFileSync(preflight.paths.manifest)
  const artifact = readFileSync(preflight.paths.artifact)
  if (digest(manifest) !== preflight.manifest.sha256 || digest(artifact) !== preflight.artifact.sha256) {
    throw new Error('candidate bytes changed after signature and receipt preflight')
  }
  const manifestJson = JSON.parse(manifest.toString('utf8'))
  const artifactUrl = manifestJson.platforms?.[preflight.platform]?.url
  if (typeof artifactUrl !== 'string') throw new Error('candidate latest.json lost its verified target platform entry')
  const routes = new Map([
    [feed.pathname, { body: manifest, type: 'application/json; charset=utf-8', count: 'manifest' }],
    [new URL(artifactUrl).pathname, { body: artifact, type: 'application/octet-stream', count: 'artifact' }],
  ])
  const metrics = { manifestRequests: 0, artifactRequests: 0, rejectedRequests: 0 }
  const sockets = new Set()
  const server = createServer({ cert: tls.cert, key: tls.key, minVersion: 'TLSv1.2' }, (requestIn, response) => {
    const path = new URL(requestIn.url || '/', feed).pathname
    const route = routes.get(path)
    if (!route || !['GET', 'HEAD'].includes(requestIn.method || '')) {
      metrics.rejectedRequests += 1
      response.writeHead(route ? 405 : 404, { 'cache-control': 'no-store' })
      response.end()
      return
    }
    metrics[`${route.count}Requests`] += 1
    response.writeHead(200, {
      'cache-control': 'no-store',
      'content-length': route.body.length,
      'content-type': route.type,
      'x-content-type-options': 'nosniff',
    })
    response.end(requestIn.method === 'HEAD' ? undefined : route.body)
  })
  server.on('connection', (socket) => {
    sockets.add(socket)
    socket.once('close', () => sockets.delete(socket))
  })
  server.on('clientError', (_error, socket) => socket.destroy())
  await listen(server, Number(feed.port))
  try {
    const probe = await requestTrustedBytes(feed, tls.ca)
    if (probe.status !== 200 || !probe.body.equals(manifest)) throw new Error('trusted HTTPS probe did not return the verified latest.json bytes')
    // The mandatory server self-probe proves TLS before launch; it is not app
    // evidence and must not satisfy the later "installed app requested feed"
    // receipt condition.
    metrics.manifestRequests = 0
  } catch (error) {
    await closeServer(server, sockets).catch(() => {})
    throw new Error(`trusted HTTPS feed probe failed: ${error.message || String(error)}`)
  }
  return {
    url: feed.toString(),
    metrics,
    close: async () => closeServer(server, sockets),
  }
}
