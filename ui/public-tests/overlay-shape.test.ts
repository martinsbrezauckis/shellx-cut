import assert from 'node:assert/strict'
import { overlayBoxStyle } from '../src/panels/Preview/composite'

const rect = { x: 0.1, y: 0.2, scale: 0.25, opacity: 1 }
assert.deepEqual(overlayBoxStyle(rect), { left: '10.0000%', top: '20.0000%', width: '25.0000%', height: '25.0000%', opacity: 1 })
const circle = overlayBoxStyle({ ...rect, shape: 'circle' } as any)
assert.equal(circle.borderRadius, '50%')
assert.equal(circle.aspectRatio, '1')
assert.equal(circle.height, 'auto')
assert.equal(circle.objectFit, 'cover')
const rounded = overlayBoxStyle({ ...rect, shape: 'rounded', radius: 0.2 } as any)
assert.equal(rounded.borderRadius, '20%')
console.log('PASS overlay-shape: rect unchanged; circle square/cover; rounded radius')
