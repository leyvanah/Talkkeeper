(() => {
  const root = document.documentElement;
  const storageKey = 'talkkeeper-site-theme';
  const systemTheme = window.matchMedia('(prefers-color-scheme: dark)');
  const modes = ['auto', 'light', 'dark'];
  const labels = { auto: 'автоматически', light: 'светлая', dark: 'тёмная' };
  let mode = 'auto';
  try {
    const saved = localStorage.getItem(storageKey);
    if (modes.includes(saved)) mode = saved;
  } catch { /* The switch also works when persistent storage is unavailable. */ }
  function apply() {
    root.dataset.themeMode = mode;
    root.dataset.theme = mode === 'auto' ? (systemTheme.matches ? 'dark' : 'light') : mode;
  }
  apply();
  systemTheme.addEventListener('change', () => { if (mode === 'auto') apply(); });
  document.addEventListener('DOMContentLoaded', () => {
    const button = document.getElementById('theme-toggle');
    if (!button) return;
    function updateButton() {
      const next = modes[(modes.indexOf(mode) + 1) % modes.length];
      const nextAction = next === 'auto' ? 'Включить автоматический режим' : `Включить ${next === 'light' ? 'светлую' : 'тёмную'} тему`;
      button.setAttribute('aria-label', `Тема: ${labels[mode]}. ${nextAction}`);
      button.title = `Тема: ${labels[mode]}. ${nextAction}`;
    }
    updateButton();
    button.addEventListener('click', () => {
      mode = modes[(modes.indexOf(mode) + 1) % modes.length];
      apply();
      try { localStorage.setItem(storageKey, mode); } catch { }
      updateButton();
    });
  });
})();
