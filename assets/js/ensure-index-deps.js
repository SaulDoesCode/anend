var deps = {
  done: !1,
  queue: new Set(),
  whenDone: fn => deps.done ? fn() : deps.queue.add(fn),
  donedone: () => deps.queue.forEach(fn => fn())
};{
const lsnOnce = (l, e, fn, ops = {}) => l.addEventListener(e, fn, Object.assign({once: !0}, ops))
,appendOnload = (child, head = document.head) => head ? head.appendChild(child) : lsnOnce(window, 'DOMContentLoaded', () => appendOnload(child))
,fetchScript = (src, onload) => {
  const script = document.createElement('script'); script.src = src
  if (onload) lsnOnce(script, 'load', e => onload(!0, e))
  document.body ? document.body.appendChild(script) : document.body.appendChild(script)
}
,checkDep = (dep, fallback, done) => dep != null ? done(!0) : fetchScript(fallback, done)
,checkDeps = (deps, whenDone, timeout = 8000) => {
  const fail = (fail = deps.size > 0) => {if (fail) throw new Error('could not satisfy all the dependencies, this load has failed')}
  (deps=new Set(deps)).forEach(d => checkDep(d[0],d[1],k=>{k?deps.delete(d):fail(!0);if(!deps.size)whenDone()}))
  setTimeout(() => fail(), timeout)
}
checkDeps([
  [window.rilti, '/js/rilti.min.js'],
  [window.dayjs, '/js/dayjs.min.js'],
  [window.dayjs_plugin_relativeTime, '/js/relativeTime.js'],
  [window.msgpack, '/js/msgpack.min.js'],
  [window.haal, '/js/haal.min.js'],
], () => deps.donedone(deps.done = !0))}