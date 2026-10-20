const isFn = fn => fn instanceof Function, isRA = Array.isArray, fltnr = r => r.reduce((cc, v) => cc.concat(isRA(v) ? fltnr(v) : v), []),
rendr = (e, rndrbl) => {
  if (rndrbl == null) return
  if (isRA(rndrbl)) e.append(...rndrbl.map(r => {
    while (isFn(r)) r = r(e)
    if (isRA(r)) {
      rendr(e, r)
      return
    }
    if (r == e) return
    if (r instanceof String) return document.createTextNode(r)
    return r
  }).filter(r => r != null))
  else if (rndrbl instanceof Promise) return rndrbl.then(r => rendr(e, r))
  if (isFn(rndrbl)) rndrbl = rndrbl(e)
  if (rndrbl instanceof String) rndrbl = document.createTextNode(rndrbl)
  if (rndrbl instanceof Node) e.append(rndrbl)
},
d = new Proxy((t, fn, ...children) => {
    let e = document.createElement(t)
    if (isFn(fn)) { fn = fn(e) }
    if (fn instanceof Date) fn = formatTimestamp(+fn)
    if (typeof fn == 'string') e.append(document.createTextNode(fn))
    if (isRA(fn)) children = [...fn, ...children]
    else if (fn instanceof Node) children = [fn, ...children]
    rendr(e, children)
    return e
}, {
  get(og, t) {
    if (t[0] == '$') return document.querySelectorAll(t.slice(1))
    if (typeof og[t] == 'function') return og[t]
    return new Proxy((...c) => og(t, ...c), {
      get(og, c) {
        let cl = [], attrs = [], evts = {}, fns = [], pd2 = new Proxy((...c) => og(e => {
          cl.length && e.classList.add(...cl)
          attrs.forEach(({key, value}) => e.setAttribute(key, value))
          for (const evt in evts) evts[evt](e)
          fns.forEach(f => f(e))
          cl = [], attrs = [], evts = {}, fns = []
          return e
        }, ...c), {get:(_, c) => handle(c)})
        const handle = c => {
          if (typeof c == 'symbol') {}
          else if (c == 'attr') return (key, value) => (typeof key == "object" ? Object.entries(key).forEach(([key, value]) => attrs.push({key, value})) : attrs.push({key, value}), pd2)
          else if (c == 'src') return src => (fns.push(e => e.setAttribute('src', src)), pd2)
          else if (c == 'css') return (css = {}, v) => (fns.push(e => v != void 0 ? e.style[css] = v : Object.assign(e.style, css)), pd2)
          else if (c[0] == 'o' && c[1] == 'n') {
            c = c.slice(2)
            return (f, ops = {}) => (evts[c] = e => e.addEventListener(c, ev => f(ev, e), ops), pd2)
          } else if (c == '_') return f => (fns.push(f), pd2)
          else if (c == 'own') return (...args) => (fns.push(e => rendr(e, args)), pd2)
          else if (c == 'at') return h => (fns.push(e => h.append(e)), pd2)
          else if (c[0] == '$') {
            c = c.slice(1)
            if (c.match(/[A-Z]/)) c = '.' + c.replace(/[A-Z]/g, l => `-${l.toLowerCase()}`)
            fns.push(e => rendr(document.querySelector(c), e))
            return pd2
          } else {
            c = c.replace(/[A-Z]/g, l => `-${l.toLowerCase()}`)
            cl.push(c)
          }
          return pd2
        }
        return handle(c)
      }
    })
  }
}), isNum = o => typeof o === 'number' && !isNaN(o),
{div, span, article, button, section, input, textarea, nav, header, main, p, pre, b, hr, h1, h2, h3, h4, h5, script} = d,
html = (input, host, r) => {
  if (isFn(input)) input = input(host)
  if (isNum(input)) input = String(input)
  r = input instanceof Node ? input :
    typeof input === 'string' ?
        Array.from(document.createRange().createContextualFragment(input).childNodes) :
    typeof input.map == 'function' ?
        input.map(i => html(i, host)) :
    new Error('html: unrndrbl input')
  if (r instanceof Error) throw r; return r
},frag = inner => inner!=void 0? html(inner) : document.createDocumentFragment(),
ifnz = (a, ...fnz) => fnz.split(',').map(fn => a[fn.trim()].call(a)),
styleOn = (host, css) => {
  if (typeof host == 'string') host = document.body.querySelector(host)
  const style = document.createElement('style')
  style.innerHTML = css
  host.append(style)
  return style
}, Style = new Proxy((strs, ...values) => {
  let style = document.querySelector('style') || document.createElement('style'),
    innerHTML = style.innerHTML || ''
  if (!document.querySelector('style')) document.body.appendChild(style)
  for (let i = 0; i < strs.length; i++) {
    innerHTML += strs[i]
    if (i < values.length) {
      let value = values[i]
      if (isFn(value)) value = value()
      innerHTML += value
    }
  }
  if (!style.$before) style.$before = []
  if (!style.$before.includes(innerHTML)) {
    Object.assign(style, {innerHTML})
    style.$before.push(innerHTML)
  }
  return Style
}, {get(S, cl) {
  const style = document.querySelector('style') || document.createElement('style')
  if (!document.querySelector('style')) document.body.appendChild(style)
  return (strs, ...values) => {
    let innerHTML = ''
    for (let i = 0; i < strs.length; i++) {
      innerHTML += strs[i]
      if (i < values.length) {
        let value = values[i]
        if (isFn(value)) value = value()
        innerHTML += value
      }
    }
    if (!(cl in S)) {
      S[cl] = true
      Object.assign(style, {innerHTML: (style.innerHTML || '') + `\n.${cl} {${innerHTML}}`})
    }
    return Style
  }
}}), tokenize = text => text.toLowerCase().replace(/[^a-z\s\.A-Z\-]/g, '').split(/\s+/).filter(s => s.length),
months = 'January February March April May June July August September October November December'.split(' '),
insertString = (original, strToInsert, index) => index < 0 || index > original.length ? original : original.slice(0, index) + strToInsert + original.slice(index),
formatTimestamp = (timestamp, offset = 0) => {
    const date = new Date(Number(timestamp) + offset),
    hour = date.getHours().toString().padStart(2, '0'),
    minutes = date.getMinutes().toString().padStart(2, '0'),
    day = date.getDate(),
    month = months[date.getMonth()],
    year = date.getFullYear()
    return `${year} ${month} ${day} at ${hour}:${minutes}`
};console.log('buenam gujuu'); const nbj = () => Object.create(null), cN = (fn, ...w) => w.map(a => fn(a)),
pstjsn = async (u, o, qua = 'json', intercept = r => r) => (await intercept(await fetch(u, {
  method: 'POST', body: JSON.stringify(o),
  headers: {'Content-Type': 'application/json'}
}))[qua]()),
sha256 = async str => Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(str)))).map(b => b.toString(16).padStart(2, '0')).join(''),
playWav = (url, au = new Audio(url)) => (au.play().then(_=>{}).catch(async error => {console.error("Playback failed:", error)}), au), Wv = n => `waves/${n}.wav`, enwave = async (body, name = sha256(body), nomore) => {
  try {
    const wav = (await fetch(Wv(name = await name))).ok ? name : 
      await (await fetch('http://localhost:5000/enwave', {method: "POST", header: { 'Content-Type': 'text/plain', 'Access-Control-Allow-Origin': '*' }, body})).json()
      return playWav(Wv(wav))
  } catch(e) {
    await firecmd('nohup', './kitty.sh', './')
    if (!nomore) setTimeout(() => enwave(body, name, 1), 5000)
  }
},serializeListOfLists = async data => {
  if (data instanceof Promise) data = await data
  const view = new DataView(new ArrayBuffer(4 + data.reduce((sum, v) => sum + 4 + v.length, 0)))
  let offset = 0
  view.setUint32(offset, data.length)
  offset += 4
  for (const inner of data) {
    view.setUint32(offset, inner.length)
    offset += 4
    new Uint8Array(view.buffer).set(inner, offset)
    offset += inner.length
  }
  return new Uint8Array(view.buffer)
}, deserializeListOfLists = async bytes => {
  if (bytes instanceof Promise) bytes = await bytes
  if (bytes.length < 4) return [bytes]
  let cursor = 0
  const result = [], bl = bytes.length, view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength), count = view.getUint32(cursor)
  cursor += 4
  for (let i = 0; i < count; i++) {
    if (cursor + 4 > bl) break
    const len = view.getUint32(cursor)
    cursor += 4
    if (cursor + len > bl) break
    result.push(bytes.slice(cursor, cursor + len))
    cursor += len
  }
  return result
}, firecmd = async (body, args, l) => await (await fetch('/cmd'+(l || args ? '?' : '')+(l ? 'l='+l : '')+(args && l ? '&' : '')+(args ? 'args='+args : ''), {method: 'POST', headers: {'Content-Type': 'text/plain'}, body})).text(),
rleEncode = lengths => {
  let i = 0, out = []
  while (i < lengths.length) {
    let run = 1, val = lengths[i]
    while (i + run < lengths.length && lengths[i + run] === val && run < 255) run++
    out.push(run, val & 0xff, (val >> 8) & 0xff)
    i += run
  }
  return new Uint8Array(out)
}, rleDecode = (data, count) => {
  let oi = 0, di = 0, out = new Uint16Array(count)
  while (oi < count && di < data.length) {
    const run = data[di], val = data[di + 1] | (data[di + 2] << 8)
    di += 3
    for (let j = 0; j < run; j++) if (oi < count) out[oi++] = val
  }
  return out
}, kraal = map => {
  const enc = new TextEncoder(), entries = (map instanceof Map ? Array.from(map.entries()) : Object.entries(map)).map(([k, v]) => [
    typeof k === 'string' ? enc.encode(k) : k,
    typeof v === 'string' ? enc.encode(v) : v
  ]),/* .sort((a, b) => { for (let i = 0; i < Math.min(a[0].length, b[0].length); i++) { if (a[0][i] !== b[0][i]) return a[0][i] - b[0][i]; } return a[0].length - b[0].length; });*/
  n = entries.length, keyLens = new Uint16Array(n), valLens = new Uint16Array(n)
  let totalKeyLen = 0, totalValLen = 0
  for (let i = 0; i < n; i++) {
    keyLens[i] = entries[i][0].length
    valLens[i] = entries[i][1].length
    totalKeyLen += keyLens[i]
    totalValLen += valLens[i]
  }
  const kdir = rleEncode(keyLens), vdir = rleEncode(valLens), totalSize = 16 + kdir.length + totalKeyLen + 4 + vdir.length + totalValLen, buffer = new Uint8Array(totalSize), view = new DataView(buffer.buffer)
  view.setUint32(0, n, true)
  view.setUint32(4, totalKeyLen, true)
  view.setUint32(8, totalValLen, true)
  view.setUint32(12, kdir.length, true)
  let p = 16
  buffer.set(kdir, p)
  p += kdir.length
  for (let e of entries) { buffer.set(e[0], p); p += e[0].length }
  view.setUint32(p, vdir.length, true); p += 4;
  buffer.set(vdir, p)
  p += vdir.length
  for (let e of entries) { buffer.set(e[1], p); p += e[1].length }
  return buffer
}, ontkraal = (data, txtValues = true) => {
  const view = new DataView(data.buffer, data.byteOffset, data.byteLength), n = view.getUint32(0, true), keyBlobLen = view.getUint32(4, true), kdirLen = view.getUint32(12, true), kdirStart = 16, keyBlobStart = kdirStart + kdirLen, keyLens = rleDecode(data.subarray(kdirStart, keyBlobStart), n), keys = []
  let kp = keyBlobStart
  for (let i = 0; i < n; i++) {
    keys.push(data.subarray(kp, kp + keyLens[i]))
    kp += keyLens[i]
  }
  const vdirLenOffset = keyBlobStart + keyBlobLen, vdirLen = view.getUint32(vdirLenOffset, true), vdirStart = vdirLenOffset + 4, valBlobStart = vdirStart + vdirLen, valLens = rleDecode(data.subarray(vdirStart, valBlobStart), n), result = new Map()
  let vp = valBlobStart
  for (let i = 0; i < n; i++) {
    result.set(keys[i], txtValues ? new TextDecoder().decode(data.subarray(vp, vp + valLens[i])): data.subarray(vp, vp + valLens[i]))
    vp += valLens[i]
  }
  return result
},recu = async stopPoint => {
  const stream = await navigator.mediaDevices.getUserMedia({ audio: true }), mediaRecorder = new MediaRecorder(stream), theWave = new Promise(async ok => {
    let audioChunks = []
    mediaRecorder.ondataavailable = ({data}) => audioChunks.push(data)
    mediaRecorder.onstop = _=> {
      const audioBlob = new Blob(audioChunks, { type: 'audio/ogg' }), audioUrl = URL.createObjectURL(audioBlob), audio = new Audio(audioUrl)
      ok(audio)
      audio.play()
      d.a.attr({href: audioUrl})._(a => a.download = 'recording_'+Math.round(Math.random()*10002)+'.ogg').onclick((e, a) => a.remove()).$body().click()
    }
    if (stopPoint) stopPoint = setTimeout(() => mediaRecorder.stop(), stopPoint)
  })
  mediaRecorder.start()
  return {stream, mediaRecorder, async stop() { (mediaRecorder.stop(), clearTimeout(stopPoint), await theWave) }}
}, primeFactors = n => {
  const fnd = []
  let d = 2
  while (n > 1) {
    while (n % d === 0) { fnd.push(d); n /= d }
    d++
    if (d * d > n && n > 1) { fnd.push(n); break }
  }
  return fnd
}, logr = i => (console.log(i), i), digitSum = d => `${d}`.split('').filter(s => s.length).map(s => Number(s)).reduce((a, b) => a+b, 0),
pfR = (s, l = 32) => {
  let pf = primeFactors(s), pfr = pf.reduce((a, b) => a+b, 0), nx = -2
  const ds = digitSum(s), dp = ds*pfr, nxch = []
  while(++nx < l) {
    const xn = nxch.length ? primeFactors(nxch[nx]).reduce((a, b) => a+b, 0) * digitSum(nxch[nx]) : primeFactors(dp).reduce((a, b) => a+b, 0) * ds
    if (nxch.includes(xn)) break
    nxch.push(xn)
  }
  const final = nxch.reduce((a, b) => a+b, 0) + pfr, fds = digitSum(final)
  return `<span class="pf">${(!pf.length ? '' : pf.length == 1 ? pf[0]+' ' : pf.join(' + ') + ' = ' + pfr + ' -> ')}* ${ds} = <i onclick="term.value = ${dp}">${dp}</i>+${nxch.map(t => `<i onclick="term.value = ${t}">${t}</i>`).join('+')} = ${final} ${fds} | ${ds + fds}</span>`
}, fetchBinaryData = async (what, t = 1, op) => {
  const arrayBuffer = await (op.startsWith('gsearch') ? pstjsn('memory/'+op, what, 'arrayBuffer', r => {
    if (!r.ok) throw new Error('memory op failed: '+op)
    return r
  }) : postBytes(op, what)), entries = ontkraal(new Uint8Array(arrayBuffer))
  if (!entries.size) throw new Error('befokde rekwest seker')
  return entries
}, toBytes = async i =>
  typeof i === "string" ? new TextEncoder().encode(i) :
  typeof i == "number" ? new Uint8Array([i]) :
  i instanceof Uint8Array ? i :
  i instanceof ArrayBuffer ? new Uint8Array(i) :
  i instanceof Blob ? new Uint8Array(await i.arrayBuffer()) :
  Array.isArray(i) ? new Uint8Array(i) :
  i instanceof Promise ? await toBytes(await i) :
  (() => { throw new TypeError(Object.prototype.toString.call(i)) })(),
postBytes = async (op, i, then = 'arrayBuffer') => {
  const r = await fetch(`/memory/${op}`, { 
    method: "POST", body: await toBytes(i),
    headers: { "Content-Type": "application/octet-stream" }
  })
  if (!r.ok) throw new Error(`[${op}] ${r.status}: ${await r.text().catch(() => r.statusText)}`)
  return await r[then]()
}, wift = async (b, t = true) => (b = new Uint8Array(b instanceof Promise ? await b : b), t ? new TextDecoder().decode(b) : b),
notereq = (a, b) => {
  if (a instanceof Uint8Array && b instanceof Uint8Array) return new Uint8Array([a.length, ...a, ...b])
  const aa = a.split(',').filter(c => c != ' ').map(c => Number(c))
  return new Uint8Array([aa.length, ...aa, ...b.split(',').filter(c => c != ' ').map(c => Number(c))])
}, memory = new Proxy({ // bread & butter
  remember:(i, t) => wift(postBytes("remember", i), t),
  saturate: (i, k, n) => wift(postBytes("saturate"+(k ? '?k='+k : "")+(n ? '&n=1' : ''), i), 0),
  forget: (i, t) => wift(postBytes("forget", i), t),
  search: (i, m, t) => fetchBinaryData(i, t, "search"+(m ? "?m="+m : '')),
  gsearch: (i, r, t) => fetchBinaryData(i, t, "gsearch"+(r ? "?r" : '')), // gematria search, r = reversed
  esearch: (q, s, m, t) => fetchBinaryData(q, t, "esearch?"+(s ? "s="+s : '')+(m ? (s ? '&' : '')+"m="+m : '')), // embedding search s = sensitivity, m = saturate or not
  notes: (a, t) => fetchBinaryData(a, t, 'notes'), // new stuff
  note: (a, b) => wift(postBytes('note', notereq(a, b))),
  unnote: (a, b) => wift(postBytes('note?d', notereq(a, b))),
  enlist: (ln, i) => wift(postBytes('enlist', notereq(ln, i))),
  delist: (ln, i) => wift(postBytes('delist', notereq(ln, i))),
  list: (ln, t) => fetchBinaryData(ln, t, "list"),
  unlist: ln => deserializeListOfLists(postBytes('unlist', ln)),
  get backup() { return new Promise(async ok => ok(await (await fetch('memory/backup')).text())) },
  get count() { return new Promise(ok => fetch('/memory/count').then(r => r.json()).then(c => ok(c))) }
}, { get: (o, k) => k in o ? o[k] : o.remember(k) }),
gprice = {
  a: 1, b: 2, c: 3, d: 4, e: 5, f: 6, g: 7, h: 8, i: 9, j: 10, k: 11, l: 12, m: 13,
  n: 14, o: 15, p: 16, q: 17, r: 18, s: 19, t: 20, u: 21, v: 22, w: 23, x: 24, y: 25, z: 26
}, rprice = {
  a: 26, b: 25, c: 24, d: 23, e: 22, f: 21, g: 20, h: 19, i: 18,
  j: 17, k: 16, l: 15, m: 14, n: 13, o: 12, p: 11, q: 10, r: 9,
  s: 8, t: 7, u: 6, v: 5, w: 4, x: 3, y: 2, z: 1}, vrices = {a: 1, e: 2, o: 3, u: 4, i: 5, y: 6},
crises= {b: 1, c: 2, d: 3, f: 4, g: 5, h: 6, j: 7, k: 8, l: 9, m: 10, n: 11, p: 12, q: 13, r: 14, s: 15, t: 16, v: 16, w: 17, x: 18, z: 19},
gsum = (txt, r, sum = 0) => {
  if (r == 2) {
    r = 0
    for (let c of txt) r += rprice[c = c.toLowerCase()] || 0
    return r
  }
  if (r) {
    const w3 = r == 3
    r = 0
    let vrice = r, crise = r
    for (let c of txt) {
      r += rprice[c = c.toLowerCase()] || 0
      sum += gprice[c] || 0
      vrice += vrices[c] || 0
      crise += crises[c] || 0
    }
    return w3 ? [sum, r] : 
`(${sum} + ${r} = ${sum + r}; * ${sum * r}; *sqrt(${Math.sqrt(sum * r).toFixed(3)}); diff ${sum - r}, ${(sum+r)+Math.abs(sum - r)} ${(sum+r)+(sum - r)} ${((sum+r)+Math.abs(sum - r)) - ((sum+r)+(sum - r))}) v ${vrice} c ${crise} w ${txt.trim().split(/\s+/).length}`
  }
  for (const c of txt) sum += gprice[c.toLowerCase()] || 0
  return sum
}, Sign = {
  async in(email, password) {
    const res = await pstjsn('/signin', [email, password], 'text')
    if (res == 'signed in') localStorage.setItem('signedin', 1)
    return res
  },
  async up(moniker, email, password) {
    const res = await pstjsn('/signup', [moniker, email, password], 'text')
    if (res == 'signed up') localStorage.setItem('signedin', 1)
    return res
  },
  async out() {
    localStorage.removeItem('signedin')
    d.p.toast.$body((await (await fetch('/signout')).text()))
  }
}, fileUppies = () => div.uppies.$body._(fu => fu.innerHTML = `<h1>Upload files</h1><form action="/upload" method="post" enctype="multipart/form-data"><input type="file" name="files" multiple/><input type="submit" value="upload" /></form>`)(),
products = { insert: (name, stock, kind, description, price, imgs = [], tags = []) => typeof name == 'object' ? products.insertO(name) : pstjsn('/product', [name, stock, kind, description, price, imgs, tags], 'text'),
  async insertO({name, stock = 1, kind = 1, description = "", price, imgs = [], tags = []} = o = {}) { return await pstjsn('/product', [name, stock, kind, description, price, imgs, tags], 'text') },
  async get(name) { return (await fetch(`/product/${name}`)).json() }, fetch(o) { return pstjsn('/products', o) }
}, wordAt = (i, txt) => txt.trim().split(/\s+/)[i], bi2u8s = bi => {
  if ((bi = `${bi}`).length % 2 !== 0) bi = '0' + bi
  const len = bi.length / 2, u8s = new Uint8Array(len)
  for (let i = 0; i < len; i++) u8s[i] = parseInt(bi.slice(i * 2, i * 2 + 2), 16);return u8s},
solveRatio = (x, y, a, b) => {
  if (x === null || x === undefined) x = (b * y) / a
  else if (y === null || y === undefined) y = (a * x) / b
  else if (a === null || a === undefined) a = (b * y) / x
  else if (b === null || b === undefined) b = (a * x) / y
  return [a * x, a / b]
}