// the colours before the first paint, so the page doesn't flash sand under night (a file
// of its own rather than inline: the hub's content security policy runs only its own
// scripts)
try {
  const s = JSON.parse(localStorage.getItem('anend.colours') || 'null');
  if (s && s.preset === 'night') document.documentElement.dataset.dark = 'dark';
} catch {}
