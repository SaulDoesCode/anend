{
  const {dom, run} = rilti
  const {html, div, pre, button, article} = dom

  function getTitle (md) {
    const start = md.indexOf('#')
    if (start === -1) return ''
    let end = md.indexOf('\n')
    let pos = 0
    while (start > end && pos < 5) end = md.indexOf('\n', pos++)
    if (!md.includes('\n')) end = md.length
    return md.substring(start, end).trim().replace('#', '').trim()
  }

  rilti.component('markdown-editor', {
    props: {
      accessors: {
        title: {
          get: el => getTitle(el.value),
          set(el, val) {
            el.value = '# ' + val + (el.value.includes('\n') ? el.value.substring(0, el.value.indexOf('\n')) : el.value)
          }
        },
        content: {
          get: el => el.value.replace('# ' + el.title, '').trim(),
          set(el, val) {
            const title = el.title
            el.value = (title.length ? '# ' + el.title + '\n\n' : '') + val
          }
        },
        rendered: el => md2html(el.value),
        renderedHTML: el => md2html(el.value, true),
        viewmode: {
          get: el => el.class.viewmode,
          set(el, val) {
            el.view.clear()
            if (el.class.viewmode = val) {
              el.view.innerHTML = el.renderedHTML
              el.pad.replace(el.view)
            } else {
              el.view.replace(el.pad)
            }
            el.viewBtn.title = val ? 'edit' : 'view'
            el.viewBtn.icon.update(val ? 'edit' : 'brush')
          }
        }
      }
    },
    methods: {
      addButton: (el, ops, ...children) => button(Object.assign({}, ops), children).prependTo(el.buttons)
    },
    bind: {
      value: {
        change(val, {host}) {
          if (host.pad.innerText.trim() !== val) host.pad.innerText = val
          if (host._viewmode) host.view.html = host.rendered
        }
      }
    },
    create(el) {
      el.update = () => {
        const val = el.pad.innerText.trim()
        if (val !== el.value) {
          el.value = val
          el.emit('update')
        }
      }

      el.buttons = div.buttons(
        el.viewBtn = button.view({
          title: el.viewmode ? 'edit' : 'view',
          onclick(e) { el.viewmode = !el.viewmode }
        }, v => v.icon = icon(el.viewmode ? 'edit' : 'brush'))
      )

      el.view = article['markdown-body']()

      el.pad = pre({
        title: 'write as you wish, nb. first heading is the title',
        css: {
          display: 'block',
          whiteSpace: 'pre-wrap',
          outline: '0'
        },
        contentEditable: navigator.userAgent.includes('Chrome') ? 'plaintext-only' : 'true',
        on: {
          input: el.update,
          blur: el.update,
          focus: el.update,
          keydown: el.update
        },
        once: {
          focus(e, el) {
            setTimeout(() => el.title = '', 5000)
          }
        }
      })

      if (el.viewmode) el.viewmode = el.viewmode
    },
    mount(el) {
      run(() => el.prepend(el.buttons, el.pad))
    }
  })

}
