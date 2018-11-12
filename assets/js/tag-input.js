{
  const {component, each, dom: {div, label, input, span, button}} = rilti

  component('tag-input', {
    props: {
      max: 6,
      maxlength: 20,
      triggers: [9, 13, 188],
      tags: () => Object.create(null),
      taglist: () => [],
      input: el => input({
        type: 'text',
        on: {
          keydown(e, inpt) {
            const triggered = el.triggers.includes(e.keyCode)
            const val = el.input.value.replace(/,/g, '').trim().replace(/[\s-]+/g, '-')
            if (el.maxlength != null && val.length >= el.maxlength) {
              if (!e.ctrlKey && !triggered && e.key != null && e.key.length < 2) {
                e.preventDefault()
                console.error(`tag is too long, the max tag length is ${el.maxlength} characters`)
                app && app.error && app.error(`tag is too long, the max tag length is ${el.maxlength} characters`)
              }
              if (val.length > el.maxlength) return
            }
            if (!triggered) return
            e.preventDefault()
            el.tagFromInput(val)
          }
        }
      }),
      accessors: {
        tagstring: {
          get: el => el.taglist.join(', '),
          set (el, val) {
            el.clearTags()
            for (const v of val.split(',')) el.addTag(v.trim(), true)
            el.emit('update')
          }
        }
      }
    },
    methods: {
      eachTag (el, fn) {
        each(el.tags, (tag, name) => fn(tag, name, el))
        return el
      },
      tagFromInput(el, val = el.input.value.replace(/,/g, '').trim().replace(/[\s-]+/g, '-')) {
        el.addTag(val)
        el.input.value = ''
        return el
      },
      addTag (el, name, silent) {
        if (name == null || name.length <= 1 || el.taglist.includes(name)) return
        if (el.max != null && el.taglist.length > el.max) {
          console.error(`too many tags, remove one before adding another`)
          app && app.error && app.error(`too many tags, remove one before adding another`)
          return
        }
        if (el.maxlength != null && name.length > el.maxlength) {
          console.error(`tag is too long, the max tag length is ${el.maxlength} characters`)
          app && app.error && app.error(`tag is too long, the max tag length is ${el.maxlength} characters`)
          return
        }
        const tag = span.tag(name, button('✕')).appendTo(el)
        tag.name = name
        el.tags[name] = tag
        el.taglist = Object.keys(el.tags)
        if (!silent) el.emit('update')
        return el
      },
      addTags(el, silent, ...tags) {
        if (typeof silent === 'string') {
          tags.unshift(silent)
          silent = false
        }
        for (const tag of rilti.flatten(tags)) el.addTag(tag, true)
        if (!silent) el.emit('update')
        return el
      },
      removeTag (el, name, silent) {
        const tag = el.tags[name]
        if (tag) {
          tag.remove()
          delete el.tags[name]
          el.taglist = Object.keys(el.tags)
          if (!silent) el.emit('update')
        }
        return el
      },
      clearTags(el, silent) {
        el.eachTag((_, name) => el.removeTag(name, true))
        el.$children.forEach(child => {
          if (child.matches('.tag')) child.remove()
        })
        if (!silent) {
          el.emit('clear')
          el.emit('update')
        }
        return el
      }
    },
    on: {
      click(e, el) {
        e.target.matches('tag-input > .tag > button')
          ? el.removeTag(e.target.parentNode.name) : el.input.focus()
      }
    },
    create(el) {
      div(label['icon-tag']({title: 'type a tag, press enter'}), el.input).render(el)
    }
  })
}