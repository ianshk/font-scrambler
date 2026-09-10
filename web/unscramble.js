// unscramble.js — browser-side mirror of font-scrambler's `src/scramble.rs`.
//
// ⚠ KEEP IN SYNC: every rule here has an exact counterpart in the Rust tool.
// If you change the keystream, region handling or a scheme there, change it
// here too and regenerate the sample. The three invariants (see scramble.rs):
//   1. `preserveMagic` keeps the 4-byte `wOF2` signature in the clear.
//   2. All transforms operate on the region AFTER the magic, indexed from 0.
//   3. `xor`/`rev` are their own inverse; `swap` un-does a rotate-left with a
//      rotate-right (that is what `descramble` performs here).

const MAGIC_LEN = 4

// xorshift32 seed, matching Rust `seed_of`: key 0 remaps to a nonzero state.
function seedOf(key) {
  const k = key >>> 0
  return k === 0 ? 0x9e3779b9 : k
}

// Advance state, return low byte. `>>> 0` mirrors Rust's u32 wrapping.
function keystreamNext(state) {
  let s = state
  s ^= s << 13
  s >>>= 0
  s ^= s >>> 17
  s >>>= 0
  s ^= s << 5
  s >>>= 0
  return { s, byte: s & 0xff }
}

/**
 * Reverse a scramble in place on a copy of `bytes`.
 * @param {Uint8Array} bytes  the scrambled WOFF2 as fetched
 * @param {{scheme:'xor'|'rev'|'swap', key:number|string, preserveMagic?:boolean}} cfg
 * @returns {Uint8Array} the reconstructed, valid WOFF2
 */
export function descramble(bytes, cfg) {
  const scheme = cfg.scheme || 'xor'
  const key = parseKey(cfg.key)
  const preserve = cfg.preserveMagic !== false // default true, matches the tool
  const out = bytes.slice() // never mutate the fetched buffer
  const start = preserve ? MAGIC_LEN : 0
  const len = out.length - start

  if (scheme === 'xor') {
    let s = seedOf(key)
    for (let i = start; i < out.length; i++) {
      const step = keystreamNext(s)
      s = step.s
      out[i] ^= step.byte
    }
  } else if (scheme === 'rev') {
    const tail = out.subarray(start)
    tail.reverse()
  } else if (scheme === 'swap') {
    // Inverse of the tool's rotate-left by len/2: rotate right by the same k.
    const k = ((len / 2) | 0) % (len || 1)
    const region = out.subarray(start)
    const rotated = new Uint8Array(len)
    for (let i = 0; i < len; i++) rotated[(i + k) % len] = region[i]
    out.set(rotated, start)
  } else {
    throw new Error(`unknown scheme: ${scheme}`)
  }
  return out
}

// `--key` is a hex ("0x…") or decimal string; the CLI default is 0x9e3779b9.
export function parseKey(key) {
  if (typeof key === 'number') return key >>> 0
  if (typeof key === 'string') {
    const s = key.trim()
    return s.startsWith('0x') || s.startsWith('0X')
      ? parseInt(s, 16) >>> 0
      : parseInt(s, 10) >>> 0
  }
  return 0
}

/**
 * Fetch a scrambled WOFF2, reconstruct it in memory, and register it as a
 * `@font-face` via a Blob URL. The real font never touches the disk and is not
 * reachable by the browser's preload scanner; that is the whole trade-off.
 *
 * `cfg` ({ scheme, key, preserveMagic }) MUST be supplied by the caller. The
 * key is never served next to the font: in the demo it comes from the key
 * input, in production from your own (authenticated) delivery path. The browser
 * always ends up holding the real key, because it has to, so this raises the bar
 * against casual theft; it is not DRM.
 *
 * @returns {Promise<FontFace>} the loaded face
 */
export async function loadScrambledFont(url, family, cfg) {
  if (!cfg) throw new Error('loadScrambledFont requires a cfg { scheme, key, preserveMagic }')
  const res = await fetch(url)
  if (!res.ok) throw new Error(`fetch ${url}: HTTP ${res.status}`)
  const scrambled = new Uint8Array(await res.arrayBuffer())

  const t0 = performance.now()
  const woff2 = descramble(scrambled, cfg)
  const ms = performance.now() - t0

  const blob = new Blob([woff2], { type: 'font/woff2' })
  const face = new FontFace(family, `url(${URL.createObjectURL(blob)})`)
  await face.load()
  document.fonts.add(face)
  face._unscrambleMs = ms // exposed for the demo's timing readout
  face._bytes = scrambled.length // served file size (== reconstructed size)
  return face
}
