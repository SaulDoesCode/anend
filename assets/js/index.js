const app = rilti.emitter()
{
const {dom, $} = rilti
const {a, div, button, br, img, h1, h2, h3, h4, input, label, span, section, aside, article, header, footer, html} = dom
/*
const converter = new showdown.Converter({openLinksInNewWindow: true, tasklists: true})
const md2html = (md, plain) => plain ? converter.makeHtml(md) : html(converter.makeHtml(md))
*/

const isEmail = email => isEmail.re.test(String(email).toLowerCase())
isEmail.re = /^(([^<>()\[\]\\.,;:\s@"]+(\.[^<>()\[\]\\.,;:\s@"]+)*)|(".+"))@((\[[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\])|(([a-zA-Z\-0-9]+\.)+[a-zA-Z]{2,}))$/

const isUsername = username => isUsername.re.test(String(username))
isUsername.re = /^[a-zA-Z0-9._-]{3,50}$/

const checkUsername = username => new Promise((resolve, reject) => {
  if (!isUsername(username)) return reject(new Error("couldn't check username"))
  haal('/check-username/' + username)(res => {
    !res.ok || !(res.out && ('ok' in res.out)) ? reject(res) : resolve(res.out)
    console.log(`The username ${username} is: `, res.out)
  })
})

const authenticate = (email, username) => new Promise((resolve, reject) => {
  if (!isEmail(email)) return reject(new Error('email is malformed'))
  if (!isUsername(username)) return reject(new Error('username is malformed'))
  authenticate.busy = true
  checkUsername(username).then(out => {
    if (out.ok) console.log('returing user')
      console.log(`Awaiting Authentication for ${username}`)
      console.time('awaiting authentication')
      haal.post('/auth', {body: {email, username}})(res => {
        console.log(`The verdict is: `, res.out, res)
        console.timeEnd('awaiting authentication')
        resolve(res)
        authenticate.busy = false
      })
  }, res => {
    reject(res)
    authenticate.busy = false
  })
})

const authbtn = section.authbtn.icon['ion-md-lock']({
  $: 'header > div.auth',
  props: {open: false},
  onclick(e, el) {
    const open = el.open = !el.open
    el.class({
      'ion-md-lock': !open,
      'ion-md-unlock': open
    })
    open ? authform.appendTo('header > div.auth') : authform.remove()
  }
})

const authform = section.auth(
  div.inputs(
    input({
      type: "checkbox",
      name: "ignore_the_starman_enforcing_anti_spam",
      value: "1",
      attr: {
        style: "display:none !important",
        tabindex: "-1",
        autocomplete: "off"
      }
    }),
    div.username(
      label({attr: {for:'username'}}, 'username'),
      authenticate.username = input({
        id: "username",
        type: 'text',
        name: 'username',
        title: 'username',
        autocomplete: 'username',
        placeholder: ' ',
        pattern: '[a-zA-Z0-9._-]{3,50}',
        required: 'required'
      })
    ),
    div.email(
      label({attr: {for: 'email'}}, 'email'),
      authenticate.email = input({
        id: 'email',
        type: 'email',
        name: 'email',
        title: 'email',
        autocomplete: 'email',
        placeholder: ' ',
        required: 'required'
      })
    )
  ),
  authenticate.button = button.submit({
    onclick(e) {
      if (!authenticate.busy) {
        authenticate(authenticate.email.value.trim(), authenticate.username.value.trim()).then(out => {
          authform.remove()
          console.log(`an email should have gone through: `, out)
        }, res => {
          authform.remove()
          console.error(`authentication failed: `, res)
        })
      }
    }
  }, 'Go')
)

const fetchWrits = (page = 0, fn, count = 15) => {
  haal(`/writlist/${page}/${count}`)(res => {
    if (res.ok && res.out && !res.out.err) {
      fn(res.out)
    } else {
      console.error('fetchWrits error: ', res)
    }
  })
}

const writlist = aside.writlist({
  $: 'body > main',
  onclick({target}) {
    if (target.matches('.writ') || (target = target.parentElement).matches('.writ')) {
      if (app.activeWrit && app.activeWrit._key === target._key) return
      app.emit.activeWrit(app.activeWrit = app.writs[target._key])
    }
  }
})
const writdisplay = section.writdisplay({
  $: 'body > main',
}, el => [
  header(
    el.$title = h1(),
    el.$created = span.created(),
    span('/'),
    el.$author = span.author()
  ),
  el.$content = article['markdown-body'](),
  footer(
    el.$tags = div.tags()
  ),
  el.$injection = div.injection()
])

app.on.activeWrit(w => {
  const {$title, $created, $author, $content, $tags} = writdisplay
  $title.txt = w.title
  $created.txt = dayjs(w.created).format('DD MMMM YYYY | hh:mm')
  $author.txt = w.author
  $content.html = w.content
  if (w.injection) {
    el.$injection.innerHTML = w.injection
  }
  $tags.clear().append(w.tags.map(t => span.tag(t)))
})

if (!app.writs) {
  app.writs = Object.create(null)
  app.eachWrit = fn => {
    for (const key in app.writs) {
      fn(app.writs[key], key, app.writs)
    }
  }
}
const populateWritlist = (page = 0, count = 15) => {
  fetchWrits(page, writs => {
    for (const w of writs) {
      app.writs[w._key] = w
      const li = div.writ({$: writlist},
        span.title(w.title),
        span.created(dayjs(w.created).fromNow())
      )
      li._key = w._key
    }
    if (!app.activeWrit) {
      const keys = Object.keys(app.writs)
      app.emit.activeWrit(app.activeWrit = app.writs[keys[keys.length - 1]])
    }
  }, count)
}

populateWritlist(0)

app.footer = footer({$: 'body'})

const footerSection = (name, ...parts) => section['foot-section']({
  $: app.footer,
  cycle: {
    mount: () => rilti.run(() => {
      document.body.style.marginBottom = app.footer.offsetHeight + 'px'
    })
  }
},
  header(name),
  div(
    ...parts
  )
)

footerSection('external', 
    a.external.github({href: 'https://github.com/SaulDoesCode'},
      span('Github'),
      'SaulDoesCode'
    ),
    a.external.email({href: 'mailto:saul@grimstack.io'},
      span('email'),
      'saul@grimstack.io'
    )
)

footerSection('support us',
    div.supportus.digitalocean(
      span(`
        By signing up for your own servers/websites/blogs at DigitalOcean
        using the `,
        a({href: 'https://m.do.co/c/6564219d6c9a'}, 'referal link'),
        `, you can keep this site running for a whole month.`
      ),
      a(
        {href: 'https://m.do.co/c/6564219d6c9a'},
        button('Get it')
      )
    )
)


}

