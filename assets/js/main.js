{ /* global localStorage fetch */
  const b = document.body

  const cacheScript = (
    url,
    fn,
    fresh = (
      !localStorage.getItem('fresh') ||
      location.host.includes('localhost')
    )
  ) => {
    const cached = localStorage[url]
    if (cached != null && !fresh) return fn(cached)
    fetch(url).then(r => r.text()).then(src => {
      localStorage.setItem(url, src)
      fn(src)
    })
  }

  const see = () => {
    b.textContent = ''
    fetch('/mainview.html').then(r => r.text()).then(v => { 
      b.innerHTML = v
      if (localStorage.getItem('cookie-ok')) {
        document.querySelector('.cookie-toast').remove()
      }
      cacheScript(
        location.host.includes('localhost') ?
        'http://localhost:2018/dist/rilti.js':
        'https://rawgit.com/SaulDoesCode/rilti.js/experimental/dist/rilti.min.js'
        , src => {
        const script = document.createElement('script')
        script.textContent = src + ';\n;'
        cacheScript('/assets/js/view.js', src => {
          script.textContent += `\n;\nrilti.run(() => {\n${src}\n});\n`
          document.head.appendChild(script)
          localStorage.setItem('fresh', false)
        })
      })
    })
  }

  localStorage.getItem('see') ? see()
    : document.querySelector('div.come-see').addEventListener('click', e => {
      b.className = 'transition-view'
      see()
      setTimeout(() => { b.className = '' }, 600)
      localStorage.setItem('see', true)
    }, {once: true})
}
