// Reusable module walker for the EMS capture. Edit MODULE + LEAVES, then invoke via
// browser_run_code_unsafe({filename}). Navigates each page, screenshots to disk,
// extracts a structural summary, and records per-page /api/ calls.
async (page) => {
  // ===== EDIT THESE TWO PER MODULE =====
  const MODULE = 'Opsætning';
  // each leaf: {leaf, file}  (leaf=null => click the top-level module itself, childless)
  const LEAVES = [
    { leaf: 'Stamdata', file: 'c997-op-stamdata.png' },
    { leaf: 'Brugeradministration', file: 'c997-op-brugeradministration.png' },
    { leaf: 'Bygningselementer', file: 'c997-op-bygningselementer.png' },
  ];
  // =====================================
  const BASE = '/home/sla/projects/ems_rust/frontend/docs/ems-recreation/screenshots/';
  const esc = s => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

  // per-leaf API capture
  const apiByLeaf = {};
  let current = null;
  page.on('response', r => {
    try {
      const u = r.url();
      if (/\/api\//.test(u) && current) {
        const key = r.request().method() + ' ' + u.split('?')[0].replace('https://ems.enity.io', '');
        (apiByLeaf[current] = apiByLeaf[current] || new Set()).add(key);
      }
    } catch (e) {}
  });

  const extractor = () => {
    const clean = s => { try { return (s==null?'':String(s)).trim().replace(/\s+/g,' '); } catch(e){ return ''; } };
    const vis = el => { const r = el.getBoundingClientRect(); return r.width>0 && r.height>0; };
    const main = document.querySelector('.app__content,.content,main') || document.body;
    const h = [...main.querySelectorAll('h1,h2,h3,h4,h5')].map(e=>clean(e.textContent)).filter(Boolean);
    const tables = [...main.querySelectorAll('table')].map(t=>({
      cap: clean((t.querySelector('caption')||{}).textContent).slice(0,40),
      cols: [...t.querySelectorAll('thead th')].map(e=>clean(e.textContent)).filter(Boolean).slice(0,14),
      rows: t.querySelectorAll('tbody tr').length }));
    const charts = {
      highcharts: main.querySelectorAll('.highcharts-container').length,
      canvas: main.querySelectorAll('canvas').length,
      map: main.querySelectorAll('.leaflet-container, .gm-style, [class*="map"]').length };
    const controls = [...main.querySelectorAll('select,[role="combobox"],[class*="dropdown"],input[type="date"],[class*="datepicker"],[class*="toggle"],[class*="switch"]')].filter(vis).map(e=>clean(e.getAttribute('placeholder')||e.textContent).slice(0,35)).filter(Boolean);
    const inputs = [...main.querySelectorAll('input:not([type=hidden]),textarea')].filter(vis).map(e=>clean(e.getAttribute('placeholder')||e.getAttribute('name')||e.getAttribute('aria-label')||e.type).slice(0,30)).filter(Boolean);
    const tabs = [...main.querySelectorAll('[role="tab"],[class*="tab__"],[class*="nav-tab"]')].filter(vis).map(e=>clean(e.textContent)).filter(Boolean);
    const buttons = [...main.querySelectorAll('button,a[class*="btn"]')].filter(vis).map(e=>clean(e.textContent||e.getAttribute('aria-label')).slice(0,30)).filter(Boolean);
    const empty = ([...main.querySelectorAll('[class*="empty"],[class*="no-data"],[class*="placeholder"]')].map(e=>clean(e.textContent)).find(Boolean)||'').slice(0,90);
    return { headings:[...new Set(h)].slice(0,22), charts, tables, controls:[...new Set(controls)].slice(0,20), inputs:[...new Set(inputs)].slice(0,20), tabs:[...new Set(tabs)], buttons:[...new Set(buttons)].slice(0,24), empty };
  };

  const DASH = 'https://ems.enity.io/App/company/997/overview/dashboard';
  const modTop = () => page.locator('.navigation-item--top-level:visible')
    .filter({ has: page.locator('.navigation-item__title__text', { hasText: new RegExp('^'+esc(MODULE)+'$') }) }).first();
  const leafLoc = (leaf) => page.locator('.navigation-item--horizontal:not(.navigation-item--top-level):visible')
    .filter({ has: page.locator('.navigation-item__title__text', { hasText: new RegExp('^'+esc(leaf)+'$') }) });
  const results = [];
  for (const { leaf, file } of LEAVES) {
    current = leaf || MODULE;
    try {
      await page.goto(DASH); await page.waitForLoadState('networkidle',{timeout:9000}).catch(()=>{});
      await page.waitForTimeout(900);
      // menu toggles flyouts on CLICK (not hover): click the top-level module to open it.
      let opened = false;
      for (let a=0; a<4 && !opened; a++) {
        await page.mouse.move(960, 540); await page.waitForTimeout(120);
        await modTop().click().catch(()=>{});
        await page.waitForTimeout(700);
        if (!leaf) { opened = true; break; }        // childless module: the click navigates
        if (await leafLoc(leaf).count() > 0) opened = true;
      }
      if (!opened) { results.push({ leaf, err: 'flyout/leaf not visible' }); continue; }
      if (leaf) await leafLoc(leaf).first().click();
      await page.waitForLoadState('networkidle', { timeout: 9000 }).catch(()=>{});
      await page.waitForTimeout(1600);
      await page.screenshot({ path: BASE + file, fullPage: true }).catch(()=>{});
      const structure = await page.evaluate(extractor);
      results.push({ leaf: leaf||MODULE, url: page.url(), title: await page.title(), structure });
    } catch (e) {
      results.push({ leaf: leaf||MODULE, err: e.message.slice(0,80), url: page.url() });
    }
  }
  const api = {};
  for (const k of Object.keys(apiByLeaf)) api[k] = [...apiByLeaf[k]].sort();
  return JSON.stringify({ module: MODULE, results, api }, null, 1);
}