const mn = main.mn.$body(), sidecar = div.sidecar(), tryfulSearchVariations = (query, variations = ['man', 'ton', 'able', 'y', 'ly', 'ings', 'ing', 'ed', 's,', '.', '?', ':', ',', 's'], f = true, max = 50, caseful) => {
  if (max <= 0) return
  let exact = query[0] == '"' && query[0] == query[query.length - 1]
  if (exact) query = query.slice(1).slice(0, -1).trim()
  let lst = query[query.length - 1]
  if (variations.length == 0) {
    if (!variations._lc) {
      variations._lc = true
      if (query != query.toLowerCase()) query = query.toLowerCase()
    }
    if (lst == 'y') {
      if (query.endsWith('iety')) {
        query = query.slice(0, -4)
        variations.push('iation', 'iations', 'ieties')
      } else if (query.endsWith('ey')) {
        query = query.slice(0, -2)
        variations.push('ies', 'ied')
      } else {
        query = query.slice(0, -1)
        variations.push('ies', 's')
      }
    } else if (lst == 's') {
      if (query.endsWith('ngs')) {
        query = query.slice(0, -3)
        variations.push('ngies')
      } else if (!variations._sless) { 
        query = query.slice(0, -1)
        variations._sless = 1
      } else return
    } else if (lst == 'r' || lst == 'd') {
      if (!variations.erer) {
        variations.erer = 1
        variations.push(!query.endsWith('er') ? 'er' : 's', 'ences')
        variations._encs = 1
      } else if (!variations._encs) {
        variations._encs = 1
        if (!query.endsWith('ences')) variations.push('ences')
      } else return
    } else if (lst == 'm' || lst == 'n') {
      if (!variations._mic) {
        variations.push('ic')
        variations._mic = 1
      } else return
    } else if (lst == 'e') {
      if (query.endsWith('ime')) {
        query = query.slice(0, -1)
        variations.push('ing')
      } else if (query.endsWith('ane')) {
        query = query.slice(0, -1)
        variations.push('ity')
      } else return
    } else if (lst == 't') {
      if (query.endsWith('ent')) {
        query = query.slice(0, -1)
        variations.push('cy', 'ial', 'ce')
      } else return
    } else if (lst == 'l') {
      if (!variations._ll) {
        variations._ll = 1
        if (query[query.length - 2] == 'l')
          variations.push('er', 'ity')
      } else return
    } else return
  }
  try { if (f) setTimeout(async _=> (await memory.search(query.trim(), alsoSaturate.checked && max == 50 ? 1 : undefined)).forEach((v, k) => !mn.innerHTML.includes(v) && articulate(k, v)), 0)
    const clone = [...variations]; clone._lc = true
    if (!caseful) {
      let fancy = query[0].toUpperCase() + query.slice(1)
      if (query != fancy) tryfulSearchVariations(fancy, clone, 1, max - 1)
    } !exact && setTimeout(async () => {
      try { (await memory.search(query.trim()+(variations.pop() || ''))).forEach((v, k) => !mn.innerHTML.includes(v) && articulate(k, v)) }  catch(e) { console.error(e) }
      tryfulSearchVariations(query, variations, f = false, max - 1, true)
  }, 0) } catch(e) { console.error(e, 'idk') }
}, envale = (v, r) => {
    term.value=v
    esoich.checked = 0
    mn.innerHTML = `<h3>${v}<br>${pfR(v)}</h3>`
    dispense(memory.gsearch(v, r))
}, reSC = () => setTimeout(_=>{
  sidecar.innerHTML = ''
  sidecar.append(...gsum(term.value,3).map((v, i) =>span.onclick(_=>envale(v, rtb.checked = i == 1))(v)))
}, 100),
articulate = (k, v) => article.attr({title: `${k} ${gsum(v,1)}`}).ondblclick(_=>term.value = k).oncontextmenu(async e => {
  if (!e.ctrlKey) return
  e.preventDefault()
  //console.log(await memory.saturate(await memory.forget(JSON.parse('['+k+']'))), e)
  const [sm,rsm] = gsum(v,3)
  envale(Math.floor(Math.sqrt(sm * rsm)), rtb.checked ? 1 : 0)
}).onclick(e => {
  e.preventDefault()
  if (e.ctrlKey) {
    window.listCandidate = k
  } else if (e.shiftKey)  {
    window.listCandidate = undefined
    window.listId = k
  } else reSC(term.value = v)
}).onauxclick(async e => e.button == 1 && await enwave(v)).at(mn)(
  v.endsWith('.jpg') || v.endsWith(".webp") || v.endsWith('.png') ? d.img.attr({
    src: v.replace('/home/saulvdw/Desktop/sayings/weum/assets', ''),
    style: `max-width: 4.20cm; max-height: 4.20cm;`
  })() : p(v)
),
dispense = async a => ((await a).forEach((v, k) => articulate(k, v)), baroeskwidjel()),
checkers = div.checkers(), chkbx = (title, checked) => input._(c => c.checked = checked).at(checkers).attr({type: 'checkbox', title}),
gsch = chkbx('do gsearch on what is other than number', 1).onchange(_=> esoich.checked = false)(), 
rtb = chkbx('reversed ordingal gematria search')(), esoich = chkbx('esearch').onclick(_=>gsch.checked=0)(), 
alsoSaturate = chkbx('search: also saturate', 1)(), toast = msg => {
  const begone = _=> t.remove(), t = p.toast.onclick(_=>term.value = msg).ondblclick(begone).$body(msg)
  setTimeout(begone, 15000)
  return msg
}, baroeskwidjel = () => (clearTimeout(baroeskwidjel.toime), baroeskwidjel.toime = setTimeout(() => {
  let mtchs = {}
  document.querySelectorAll('main > article').forEach(art => {
    let nao = gsum(art.textContent, 3)[!rtb.checked ? 1: 0]
    if (!mtchs[nao]) mtchs[nao] = []
    mtchs[nao].push(art)
  })
  for (const m in mtchs) if (m.length >= 2) mtchs[m].forEach(art => {
      art.style.textDecoration = 'underline'
      art.style.textDecorationColor = seedCLR(m)
  });
}, 1300)),
sensitivityDial = input.sensitivity.attr({type: 'range', min: '0', max: '1', step:'0.001', value: '0.6', title: 'sensitivity dial'})(),
term = input.term.attr({type: 'text'}).onkeydown(async e => {
  if (e.key == 'Enter') {
    if (!term.value.length) return
    if (term.value == 'Enlist') {
      if (listId.length && listCandidate) await memory.enlist(listId, listCandidate)
      return
    } else if (term.value == 'List') {
      if (listId.length) console.log(await memory.list(listId))
      return
    } else if (term.value == 'Delist') {
      if (listId.length && listCandidate) await memory.delist(listId, listCandidate)
      return
    } else if (term.value == 'Unlist') {
      if (listId.length) await memory.unlist(listId)
      return
    }
    const nv = Number(term.value)
    if (isNaN(nv) && !e.shiftKey) {
      if (e.ctrlKey) {
        const v = term.value.trim(), s = gsum(v, rtb.checked && 2)
        mn.innerHTML = `<h3>${s}<br>${pfR(s)}</h3>`
        dispense(memory.gsearch(s, rtb.checked))
        articulate(toast(await memory.saturate(v, 0)).join(', '), v)
      } else try {
        mn.innerHTML = ''
        if (esoich.checked) {
          (await memory.esearch(term.value.trim(), sensitivityDial.value, alsoSaturate.checked ? 1 : undefined)).forEach((v, k) => !mn.innerHTML.includes(v) && articulate(k, v))
          gsch.checked = false
        } else if (gsch.checked) {
          esoich.checked = false
          const v = term.value.trim(), s = gsum(v, rtb.checked && 2)
          mn.innerHTML = `<h3>${s}<br>${pfR(s)}</h3>`
          dispense(memory.gsearch(s, rtb.checked))
          memory.search(v, alsoSaturate.checked ? 1 : undefined).then(these => {
            baroeskwidjel()
            for (const [id, one] of these.entries()) if (one == v) {
              setTimeout(() => !mn.innerHTML.includes(v) && articulate(id, v), 15)
              break
            }
          }).catch(async ({msg}) => {
            if (!msg.endsWith('saturated')) articulate(toast(await memory.saturate(v, 0)).join(', '), v)
          })
        } else tryfulSearchVariations(term.value.trim())
      } catch(e) { console.error(e) }
    } else {
      if (e.shiftKey) {
        mn.innerHTML = ''
        articulate(term.value, await memory.remember(JSON.parse('[' + term.value + ']')))
      } else {
        mn.innerHTML = `<h3>${nv}<br>${pfR(nv)}</h3>`
        dispense(memory.gsearch(nv, rtb.checked))
      }
    }
  } else if (e.key == 'Delete') {
    try {
      const it = JSON.parse('[' + term.value + ']')
      if (Array.isArray(it)) console.log(await memory.forget(it))
    } catch(e) { console.error("oof: ", e) }
  } else reSC()
}).$body()
term.after(sidecar, checkers, sensitivityDial)
window.onkeydown = e => {
  if (e.target == term) return
  if (e.key == 'r') rtb.checked = !rtb.checked
  else if (e.key == 'e') esoich.checked = !esoich.checked // else console.log(e.key, e.target, e)
  else if (e.key.startsWith('Arrow')) {
    if (e.key.endsWith('Up')) {} else if (e.key.endsWith('Left')) {
      envale(Number(document.querySelector('.sidecar > span:first-of-type').innerText))
    } else if (e.key.endsWith('Right')) {
      envale(Number(document.querySelector('.sidecar > span:last-of-type').innerText))
    } else if (e.key.endsWith('Down')) {}
  }
}
ogStyle = document.head.querySelector('style').innerHTML
;(window.onhashchange = _=> {
  if (location.hash == '#wm') document.head.querySelector('style').innerHTML += `
    body, .toast, main.mn, .sidecar { background-color: #fff; color: #000; }
    main.mn {
      article { background-color: inherit; color: inherit; text-shadow: 0 .5px 2px rgba(0,0,0,.45); }
      article:hover { text-shadow: 0 0 3px rgba(222,200,200,.4); }
    }`
  else document.head.querySelector('style').innerHTML = ogStyle
})();
function seedCLR(m) {
  const str = String(m)
  let hash = 0
  for (let i = 0; i < str.length; i++) {
    hash = str.charCodeAt(i) + ((hash << 5) - hash)
    hash |= 0
  }
  const hue = Math.abs(hash % 360)
  return `hsl(${hue}, 70%, 45%)`
}