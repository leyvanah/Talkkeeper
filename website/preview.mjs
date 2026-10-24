import { turns } from './preview-transcript.mjs';
const glyphs = {
  search:'<circle cx="10.8" cy="10.8" r="7.2"/><path d="m16 16 4.5 4.5"/>',
  mic:'<rect x="9" y="2" width="6" height="12" rx="3"/><path d="M5 10v2a7 7 0 0 0 14 0v-2M12 19v3m-4 0h8"/>',
  notes:'<path d="M4 3h11v18H4zM2 7h4m-4 5h4m-4 5h4m7-7 6-6 3 3-6 6-4 1z"/>',
  upload:'<path d="M12 16V3m-5 5 5-5 5 5M4 15v6h16v-6"/>',
  settings:'<path d="m9 3-1 3-3 1-2 3 2 2-1 3 3 3 3-1 2 2 3-2 3 1 2-3-1-3 2-2-2-3-3-1-1-3z"/><circle cx="12" cy="12" r="3"/>',
  table:'<rect x="3" y="3" width="18" height="18" rx="2"/><path d="M3 8h18M9 8v13"/>',
  chat:'<path d="M3 3h12v10H8l-4 3v-3H3zM11 17h5l4 3V9h-2"/>',
  download:'<path d="M12 3v12m-5-5 5 5 5-5M4 17v4h16v-4"/>',
  user:'<circle cx="12" cy="7" r="3"/><path d="M5 22v-3a7 7 0 0 1 14 0v3"/>',
  undo:'<path d="m8 5-5 5 5 5M3 10h10a6 6 0 0 1 0 12"/>',
  save:'<path d="M3 3h15l3 3v15H3zM7 3v6h10V3M7 21v-8h10v8"/>',
  copy:'<rect x="8" y="8" width="13" height="13" rx="2"/><path d="M16 5V3H3v13h2"/>',
  sparkle:'<path d="m12 3 2.5 6.5L21 12l-6.5 2.5L12 21l-2.5-6.5L3 12l6.5-2.5zM20 2v4m-2-2h4"/>',
  play:'<path d="m7 3 14 9-14 9z"/>',
};
const icon = name => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${glyphs[name]}</svg>`;
const decorative = (name,cls='') => `<span class="preview-icon ${cls}" aria-hidden="true">${icon(name)}</span>`;
export function preview() {
return `<section class="wrap demo-section" id="features" aria-label="Интерфейс Talkkeeper на вымышленной записи">
<div class="app-preview">
  <aside class="preview-rail" aria-hidden="true"><img src="/assets/talkkeeper.png" alt="" width="42" height="42"><div class="rail-actions">${decorative('search')}${decorative('mic','record-icon')}${decorative('notes','rail-tile')}${decorative('upload','rail-tile')}</div>${decorative('settings','rail-settings')}</aside>
  <div class="preview-app">
    <div class="preview-titlebar"><strong>Планирование встречи книжного клуба</strong><span class="preview-person">${icon('user')} Книжный клуб</span><span class="preview-date">1 окт., 18:00–18:03</span><div class="preview-window" aria-hidden="true">${decorative('download')}<span>···</span><span>−</span><span>□</span><span>×</span></div></div>
    <div class="preview-columns">
      <div class="preview-transcript">
        <div class="preview-toolbar" role="tablist" aria-label="Вид расшифровки"><button role="tab" id="view-chat" aria-controls="preview-chat" aria-selected="false" tabindex="-1" data-preview-layout="chat" aria-label="Реплики списком">${icon('chat')}</button><button role="tab" id="view-table" aria-controls="preview-table" aria-selected="true" data-preview-layout="table" aria-label="Реплики на временной шкале">${icon('table')}</button><span class="toolbar-divider"></span>${decorative('undo')}${decorative('undo','redo-icon')}</div>
        <div class="preview-table" id="preview-table" role="tabpanel" aria-labelledby="view-table">
          <div class="speaker-columns"><span>+</span><span>АННА</span><span>МИХАИЛ</span></div>
          <div class="transcript-grid">
            ${turns.map(turn=>`<div class="timeline-stamp">${turn.start}<span></span></div>${turn.side===1?'<div></div>':''}<article class="transcript-bubble"><div class="bubble-meta"><i></i>${turn.speaker} <span>${turn.start}–${turn.end}</span></div><p>${turn.text}</p></article>${turn.side===0?'<div></div>':''}`).join('')}
          </div>
        </div>
        <div class="preview-chat" id="preview-chat" role="tabpanel" aria-labelledby="view-chat" hidden>
          ${turns.map(turn=>`<article><span>${turn.start} · ${turn.speaker}</span><p>${turn.text}</p></article>`).join('')}
        </div>
        <div class="preview-player" aria-hidden="true">${icon('play')}<span>0:00</span><div class="player-track"><i></i></div><span>3:00</span><span>1×</span></div>
      </div>
      <div class="preview-details">
        <div class="preview-side-tabs" role="tablist" aria-label="Материалы встречи"><button id="preview-summary-tab" role="tab" aria-selected="true" aria-controls="preview-summary" data-preview-panel="summary">Саммари</button><button id="preview-notes-tab" role="tab" aria-selected="false" tabindex="-1" aria-controls="preview-notes" data-preview-panel="notes">Заметки</button><button id="preview-ai-tab" role="tab" aria-selected="false" tabindex="-1" aria-controls="preview-ai" data-preview-panel="ai">Чат</button></div>
        <div class="preview-side-content">
          <div id="preview-summary" role="tabpanel" aria-labelledby="preview-summary-tab"><div class="summary-actions"><span class="summary-action" aria-hidden="true">${icon('sparkle')} Пересоздать саммари</span><span class="summary-icons" aria-hidden="true">${decorative('save')}${decorative('copy')}</span></div><h4>Главное</h4><p>Участники спланировали следующую встречу книжного клуба. Для обсуждения выбраны три коротких рассказа. Встреча состоится 17 октября в 16:00 в библиотеке и продлится полтора часа.</p><h4 class="tasks-heading">Задачи <span>2</span></h4><div class="preview-task"><i></i><div><p>Подготовить список рассказов и вопросы к пятнице</p><span class="assignee">${icon('user')} Михаил</span></div></div><div class="preview-task"><i></i><div><p>Отправить приглашение и схему прохода в библиотеку</p><span class="assignee">${icon('user')} Анна</span></div></div><h4 class="topics-heading">Ключевые темы</h4><span class="topic-chip">Выбор книги</span><span class="topic-chip">Формат встречи</span></div>
          <div id="preview-notes" role="tabpanel" aria-labelledby="preview-notes-tab" hidden><h4>Заметки к встрече</h4><div class="preview-note"><span>0:45</span><p>Выбираем три коротких рассказа. Список и вопросы — к пятнице.</p></div><div class="preview-note"><span>1:34</span><p>Встреча 17 октября, 16:00, в библиотеке. Зал на 90 минут.</p></div></div>
          <div id="preview-ai" role="tabpanel" aria-labelledby="preview-ai-tab" hidden><div class="preview-question">Что решили подготовить до встречи?</div><p class="preview-answer">Михаил подготовит список трёх рассказов и вопросы к пятнице. Анна отправит приглашение и схему прохода. Участников попросят подтвердить участие до вторника.</p><div class="preview-chat-input" aria-hidden="true">Задайте вопрос по записи <span>↑</span></div></div>
        </div>
      </div>
    </div>
  </div>
</div></section>`;
}
