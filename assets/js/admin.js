/* global localStorage fetch */
console.time('admin init')
try { localStorage.getItem('cookie-ok') && document.querySelector('.cookie-toast').remove() } catch(e) {}
const app = rilti.emitter()
{
if (!location.hash) location.hash = 'editor'
const {$, run, dom, route} = rilti
const {html} = dom

app.conf = new Proxy('conf' in localStorage ? msgpack.decode(Uint8Array.from(localStorage.conf.split(',').map(i => parseInt(i, 10)))) : {}, {
  set (conf, key, val) {
    if (key in conf && val === conf[key]) return
    Reflect.set(conf, key, val)
    localStorage.setItem('conf', msgpack.encode(conf))
  },
  delete (conf, key) {
    Reflect.deleteProperty(conf, key)
    localStorage.setItem('conf', msgpack.encode(conf))
  }
})

run(() => {
  const MSG = document.querySelector('nav > div.msg')
  app.error = msg => {
    MSG.textContent = msg
    setTimeout(() => MSG.textContent = '', 5000)
  }
})

app.bakeWrit = (writ, fn) => haal.post('/writ', {body: writ})(res => console.log('bake writ result: ', res))

app.queryWrits = async (query = {}, fn) => {
  if (!('editormode' in query)) query.editormode = true
  if (!('includeprivate' in query)) query.includeprivate = true
  if (!('includemembersonly' in query)) query.includemembersonly = true
  if (!query.omissions) {
    query.omissions = []
  }
  try {
    const res = await haal.post('/writ-query', {body: query})()
    if (res.out.err) throw res.out.err
    console.log(`Writ Success!:`, res.out)
    if (fn) fn(res, res.ok)
    return res
  } catch(res) {
    console.error(`Writ Query Problem:`, res.err, res)
    if (fn) fn(res, res.ok)
    return res
  }
}

app.deleteWrit = (key, fn) => new Promise((resolve, reject) => {
  haal('/writ-delete/' + key)((res, out) => {
    if (!res.ok || !res.err) {
      fn && fn(out, res)
      return reject(res)
    }
    fn && fn(out, res)
    resolve(out)
  })
})

app.populateWritlist = async (page = 0, count = 100) => {
  const res = await app.queryWrits({limits: [page, count]})
  app.emit.writlistReady(app.writlist = res.out)
  return res
}

app.isWritSaveable = w => (
  'title' in w && w.title.length > 1 &&
  'markdown' in w && w.markdown.length > 1 &&
  'tags' in w && Array.isArray(w.tags) && w.tags.length
)

app.saveWrit = (w = app.activeWrit) => {
  if (!app.isWritSaveable(w)) throw new Error('cannot save invalid/incomplete writs')
  for (const key in w) if (w[key] == null || w[key] == '') delete w[key]
  return app.bakeWrit(w)
}

app.updateWrit = async w => {
  const res = await app.queryWrits({one: true, _key: w._key})
  console.log('updated writ: ', res.out)
  return rilti.merge(w, res.out)
}

app.deleteWrit = w => new Promise((resolve, reject) => haal('/writ-delete/' + w._key)((res, out) => {
  if (!res.ok) {
    console.error(`writ deletion problem:`, res.err, res)
    reject(res)
    return errfn && errfn(res.err, res)
  }
  console.log(`writ deleted:`, out)
  fn && fn(out, res)
  resolve(out)
}))

rilti.run(async () => {
const converter = new showdown.Converter({openLinksInNewWindow: true, tasklists: true})
app.md2html = (md, plain) => plain ? converter.makeHtml(md) : html(converter.makeHtml(md))
await appInit(app, dom, rilti)
console.timeEnd('admin init')
console.log('admin panel ready')
})
}

async function appInit(app, {h1, nav, div, main, section, span, aside, article, pre, input, header, button}, {dom, $, route, isEl}) {
  let editor
  let writlist
  let taginput
  let injectarea
  const btns = {}

  app.once.writlistReady(wl => {
    app.writMap = new Map()
    writlist.on.click(({target}) => {
      if (target.matches('.writ')) {
        const w = app.writMap.get(target._key)
        if (w != null && w !== app.activeWrit) {
          app.emit.editWrit(app.activeWrit = w)
        }
      }
    })
    for (const w of wl) {
      app.writMap.set(w._key, w)
      const {title} = w
      const li = div.writ({title}, title)
      li._key = w._key
      writlist.append(li)
      console.log(w.title, w)
    }
  })

  app.on.editWrit(w => {
    if (w.title != null && w.markdown != null) {
      editor.value = '# ' + w.title + '\n\n' + w.markdown
    } else {
      editor.value = ''
    }
    taginput.clearTags(true)
    if (w.tags && w.tags.length) {
      taginput.addTags(true, w.tags.filter(t => t.length && t != ''))
    }
    injectarea.value = 'injection' in w ? w.injection : ''
    if (editor.viewmode) {
      editor.viewmode = !editor.viewmode
      editor.viewmode = !editor.viewmode
    }
    btns.publish.class({published: !!w.public})
    btns.publish.title = !!w.public ? 'unpublish' : 'publish'
    btns.publish.icon.update(!!w.public ? 'eye' : 'eye-off')
  })

  app.on.save(async () => {
    if (!app.activeWrit) return
    await app.saveWrit()
    try {
      await app.updateWrit(app.activeWrit)
    } catch(e) {}
  })

  app.on.publish(async () => {
    if (!app.activeWrit) return
    app.activeWrit.public = !app.activeWrit.public
    await app.saveWrit()
    btns.publish.class({published: app.activeWrit.public})
    btns.publish.title = app.activeWrit.public ? 'unpublish' : 'publish'
    btns.publish.icon.update(app.activeWrit.public ? 'eye' : 'eye-off')
  })

  app.on.newWrit(async () => {
    if (app.activeWrit) await app.saveWrit()
    app.emit.editWrit(app.activeWrit = {})
  })

  route.whenActive('editor', async () => {
    for (let el of route.views['#editor']) {
      if (!isEl(el)) continue
      el = $(el)
      if (el.matches('markdown-editor')) editor = el
      else if (el.matches('.writlist')) writlist = el
    }

    await (async () => await rilti.componentReady(editor))()

    injectarea = editor.findOne('.meta textarea')
    taginput = editor.findOne('.meta tag-input')

    await (async () => await rilti.componentReady(taginput))()

    btns.newWrit = writlist.findOne('.new-writ')
    btns.newWrit.on.click(app.emit.newWrit)
    icon('plus-circle', {$: btns.newWrit})


    btns.publish = editor.addButton({
      class: 'publish',
      title: 'publish',
      onclick: app.emit.publish
    })
    btns.publish.icon = icon('eye', {$: btns.publish})

    btns.save = editor.addButton({
      class: 'save',
      title: 'save',
      onclick: app.emit.save
    })
    btns.save.icon = icon('save', {$: btns.save})

    editor.on.update(e => {
      if (!app.activeWrit) return
      app.activeWrit.title = editor.title
      app.activeWrit.markdown = editor.content
    })

    editor.pad.on.keydown(e => {
      if (e.ctrlKey && e.keyCode === 83 && app.activeWrit != null) {
        e.preventDefault()
        e.stopPropagation()
        app.emit.save()
      }
    })

    taginput.on.update(e => {
      if (!app.activeWrit) return
      app.activeWrit.tags = taginput.taglist
    })

    const updateInject = e => {
      const val = injectarea.value.trim()
      if (!app.activeWrit || app.activeWrit.injection === val) return
      app.activeWrit.injection = val
    }
    injectarea.on.blur(updateInject)
    injectarea.on.input(updateInject)

    await app.populateWritlist()
  }, true)

  app.restartApp = () => haal('/_updateapp')(res => {
    console.log(res)
    document.body.background = 'var(--the-mood)'
    document.body.fontSize = '3em'
    document.body.innerHTML = res.out
    setTimeout(() => location.reload(), 12000)
  })
}