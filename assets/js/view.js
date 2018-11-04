const {component, dom, isStr, isObj, merge, $, run} = rilti
const {ul, li, a, p, b, img, aside, div, section, header, h1, h2, h3, h4, main} = dom

if (!location.hash) location.hash = '#home'

component('link-list', {
  props: {
    header: () => header()
  },
  mount(ll) {
    const list = ul()
    for (const link of ll.$children) {
      if (link.href) link.attr.title = link.href
      li({$: list}, link)
    }
    ll.append(ll.header, list)
    run(() => {
      if (ll.attr.has('newtab')) {
        ll.attr({ newtab: null }).attr.newtab = true
      }
    })
  },
  attr: {
    title(ll, title) {
      ll.header.attr({title}).txt = title
      delete ll.attr.title
    },
    newtab: {
      init(ll) {
        for (const link of ll.find('a')) link.attr({ target: '_blank', rel: 'noopener'})
      },
      remove(ll) {
        for (const link of ll.find('a')) link.attr({target: null, rel: null})
      }
    }
  }
})

component('ideas-section', {
  props: {
    accessors: {
      active: {
        get (el) {
          const idea = el.findOne('idea-block.active')
          if (idea) return idea
        },
        set(el, idea) {
          el.activate(idea)
        }
      }
    }
  },
  methods: {
    activate (el, idea) {
      if (isStr(idea)) idea = el.findOne(`idea-block[name="${idea}"]`)
      if (idea = $(idea)) {
        const active = el.findOne('idea-block.active')
        if (active) active.class.active = false
        el.display.html = idea.content
        idea.class.active = true
      }
    }
  },
  create (el) {
    el.grid = dom.ideaGrid({$: el})
    el.display = dom.ideaDisplay({$: el})
    el.on.click('idea-block:not(.active)', ({target}) => el.activate(target))
  },
  mount (el) {
    run(() => {
      el.grid.append(el.find('idea-block'))
      run(() => !el.active && el.activate(el.grid.findOne('idea-block')))
    })
  }
})

component('idea-block', {
  props: {
    accessors: {
      name: (ib, val) => val == null ? ib.attr.name : ib.attr.name = val
    }
  },
  create (ib) {
    if (ib.txt.length) ib.content = ib.html
  },
  mount (ib) {
    if (ib.txt.length) ib.content = ib.html
  },
  attr: {
    name (ib, val) {
      run(() => {
        ib.txt = ib.attr.name
      })
    }
  }
})

{

  const toggleAbleOpenAttr = (config = {} , toggler = div.toggler()) => merge(config, {
    methods: {
      toggle(el, open = !el.open) {
        const event = new CustomEvent('toggle', {detail: {open}})
        el.open = event.open = open
        el.dispatchEvent(event)
      }
    },
    props: {
      toggler: el => {
        isStr(toggler) ? toggler = el.findOne(toggler) : el.append(toggler)
        toggler.on.click(e => el.toggle())
        return toggler
      },
      accessors: {
        open: {
          get: el => el.attr.has('open'),
          set: (el, open) => el.attrToggle('open', !!open)
        }
      }
    }
  })

  component('side-bar', toggleAbleOpenAttr({
    methods: {
      adjustBody(sb) {
        run(() => {
          document.body.style.width = sb.open && window.innerWidth > 760 ?
            `calc(100% - ${sb.offsetWidth}px)` : ''
        })
      }
    },
    props: {
      accessors: {
        selected: {
          get(sb) {
            const selected = sb.state.selected || sb.findOne('sb-item.selected')
            if (selected) return $(selected)
          },
          set(sb, selected) {
            selected = $(selected)
            if (selected.class.selected) return
            selected.class.selected = true
            if (sb.selected) {
              (sb.selectedLast = sb.selected).class.selected = false
            }
            sb.state.selected = selected
            const event = new CustomEvent('select')
            event.selected = selected
            event.selectedLast = sb.selectedLast
            sb.dispatchEvent(event)
          }
        }
      }
    },
    mount(el) {
      el.on.click(({target}) => {
        if (target === el || target === el()) return
        if ((target = $(target)).matches('sb-item') && !target.class.selected) {
          el.selected = target
        }
      })
      el.adjustBody()
      el.on.toggle(el.adjustBody)
    }
  }))

  component('sb-menu', toggleAbleOpenAttr({}, 'sb-menu-title'))
}

{ /* global rilti */
  const {directive, each, runAsync, $, isRenderable, isProxyNode, isFunc, isStr, on, render} = rilti

  const routes = new Map()
  routes.viewBinds = new Map()
  routes.activeBinds = new Map()

  const route = rilti.route = (name, consumer) => {
    if (name[0] !== '#') name = '#' + name

    if (isRenderable(consumer)) {
      if (consumer.tagName === 'TEMPLATE') {
        const template = consumer
        consumer = Array.from(consumer.content.childNodes)
        template.remove()
      }
      if (routes.has(name)) {
        routes.get(name).view = consumer
      } else {
        routes.set(name, {name, view: consumer})
      }
    } else if (isFunc(consumer)) {
      if (!routes.has(name)) routes.set(name, {name, consumers: new Set()})
      routes.get(name).consumers.add(consumer)
    }
    runAsync(() => route.activate())
  }
  route.viewbind = (name, host) => {
    if (!isStr(name) && !host) [host, name] = [name, false]
    if (host.tagName === 'TEMPLATE') return
    if (!isProxyNode(host)) host = $(host)
    const viewbind = (route, active) => {
      host.textContent = ''
      if ('view' in route && active) render(route.view, host)
    }
    viewbind.revoke = () => {
      if (name) {
        routes.get(name).consumers.delete(viewbind)
        routes.viewBinds.delete(host)
      } else if (routes.activeBinds.has(host)) {
        routes.activeBinds.delete(host)
      }
    }
    if (name) {
      route(name, viewbind)
      routes.viewBinds.set(host, viewbind)
    } else {
      routes.activeBinds.set(host, viewbind)
    }
    route.activate()
    return viewbind
  }
  route.revoke = route => {
    if ( (route = routes.get(route))) {
      if (route.consumers && route.consumers.size) {
        each(route.consumers, consumer => consumer.revoke && consumer.revoke())
        route.consumers.clear()
      }
      routes.delete(route.name)
    }
  }

  route.activate = (name = location.hash || '#') => {
    if (name[0] !== '#') name = '#' + name
    if (!routes.has(name) || name === routes.active) return
    if (name !== location.hash || '#') location.hash = name
    const route = routes.get(name)
    if (route.consumers && route.consumers.size) {
      each(route.consumers, consume => consume(route, true, name))
    }
    if (routes.activeBinds.size) {
      each(routes.activeBinds, bind => bind(route, true, name))
    }
    if (routes.active != null) {
      const oldroute = routes.get(routes.active)
      if (oldroute.consumers && oldroute.consumers.size) {
        each(oldroute.consumers, c => c(oldroute, false, routes.active))
      }
    }
    routes.active = name
  }

  const removeVbindRoute = el => {
    const vbind = routes.viewBinds.get(el)
    if (vbind) vbind.revoke()
  }

  directive('route', {
    init(el, val) {
      el.tagName === 'TEMPLATE' ? route(val, el) : route.viewbind(val, el)
    },
    update(el, val) {
      removeVbindRoute(el)
      route.viewbind(val, el)
    },
    remove: removeVbindRoute
  })

  directive('route-active', {
    init: el => route.viewbind(el),
    remove: removeVbindRoute
  })

  directive('route-link', {
    init(el, RLName) {
      el.state.RLL = el.on.click(e => route.activate(el.attr['route-link']))
      run(() => {
        let hash = el.attr['route-link']
        if (hash[0] !== '#') hash = '#' + hash
        if (location.hash === hash) el.click()
      })
    },
    update(el) {
      run(() => {
        let hash = el.attr['route-link']
        if (hash[0] !== '#') hash = '#' + hash
        if (location.hash === hash) el.click()
      })
    },
    remove(el) {
      el.state.RLL.off()
      state({RLName: null})
    }
  })

  on.hashchange(window, e => route.activate())
}
