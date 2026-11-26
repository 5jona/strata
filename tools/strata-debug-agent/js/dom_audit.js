// In-page L1 WebView DOM health audit executed via CDP Runtime.evaluate.
(() => {
  const vw = window.innerWidth || document.documentElement.clientWidth || 0;
  const vh = window.innerHeight || document.documentElement.clientHeight || 0;
  const docClientW = document.documentElement.clientWidth || 0;
  const rootOverflowX = document.documentElement.scrollWidth > vw + 1;
  const horizontalOverflow = document.documentElement.scrollWidth > docClientW + 1;
  const rootOverflowY = document.documentElement.scrollHeight > vh + 1;
  const overflowSamples = [];
  const horizontalOverflowSelectors = [];
  const offscreenSamples = [];
  const truncated = [];
  const smallTargets = [];
  const interactiveRects = [];
  let truncatedCount = 0;
  let smallTargetsCount = 0;

  const shortSelector = (el) => {
    let s = el.tagName.toLowerCase();
    if (el.id) s += `#${el.id}`;
    const dp = el.getAttribute && el.getAttribute('data-pochita');
    if (dp) s += `[data-pochita="${dp}"]`;
    if (typeof el.className === 'string' && el.className.trim()) {
      const cls = el.className.trim().split(/\s+/).slice(0, 2).join('.');
      if (cls) s += `.${cls}`;
    }
    return s.slice(0, 80);
  };

  const domNodeCount = document.getElementsByTagName('*').length;
  const els = Array.from(document.querySelectorAll('*')).slice(0, 600);
  for (const el of els) {
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden' || Number.parseFloat(cs.opacity) === 0) {
      continue;
    }
    const r = el.getBoundingClientRect();
    if (r.width === 0 && r.height === 0) continue;

    if (
      (el.scrollWidth > el.clientWidth + 2 && cs.overflowX !== 'auto' && cs.overflowX !== 'scroll') ||
      r.right > docClientW + 2 ||
      r.left < -2
    ) {
      if (horizontalOverflowSelectors.length < 5) {
        const sel = shortSelector(el);
        if (!horizontalOverflowSelectors.includes(sel)) {
          horizontalOverflowSelectors.push(sel);
        }
      }
    }

    if (el.scrollWidth > el.clientWidth + 2 && cs.overflowX !== 'auto' && cs.overflowX !== 'scroll') {
      if (overflowSamples.length < 8) {
        overflowSamples.push({
          tag: el.tagName.toLowerCase(),
          id: el.id || null,
          cls: typeof el.className === 'string' ? el.className.slice(0, 40) : null,
          scrollW: Math.round(el.scrollWidth),
          clientW: Math.round(el.clientWidth),
        });
      }
      const clips =
        cs.textOverflow === 'ellipsis' || cs.overflowX === 'hidden' || cs.overflow === 'hidden';
      const txt = (el.textContent || '').trim();
      const hasOwnText = el.children.length === 0 || cs.textOverflow === 'ellipsis';
      if (clips && txt && hasOwnText && el.clientWidth > 0) {
        truncatedCount++;
        if (truncated.length < 12) {
          truncated.push({
            tag: el.tagName.toLowerCase(),
            id: el.id || null,
            cls: typeof el.className === 'string' ? el.className.slice(0, 40) : null,
            text: txt.slice(0, 40),
            scrollW: Math.round(el.scrollWidth),
            clientW: Math.round(el.clientWidth),
          });
        }
      }
    }

    if ((r.right > vw + 2 || r.left < -2) && offscreenSamples.length < 8) {
      offscreenSamples.push({
        tag: el.tagName.toLowerCase(),
        left: Math.round(r.left),
        right: Math.round(r.right),
        vw,
      });
    }

    if (el.matches('button,a,input,select,[role=button]')) {
      interactiveRects.push({ tag: el.tagName.toLowerCase(), r });
      if (r.width > 0 && r.height > 0 && (r.width < 36 || r.height < 36)) {
        smallTargetsCount++;
        if (smallTargets.length < 12) {
          smallTargets.push({
            tag: el.tagName.toLowerCase(),
            id: el.id || null,
            ariaLabel: el.getAttribute('aria-label') || null,
            text: (el.textContent || '').trim().slice(0, 30) || null,
            w: Math.round(r.width),
            h: Math.round(r.height),
          });
        }
      }
    }
  }

  let overlapCount = 0;
  for (let i = 0; i < interactiveRects.length && i < 60; i++) {
    const a = interactiveRects[i].r;
    for (let j = i + 1; j < interactiveRects.length && j < 60; j++) {
      const b = interactiveRects[j].r;
      if (
        Math.min(a.right, b.right) - Math.max(a.left, b.left) > 4 &&
        Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top) > 4
      ) {
        overlapCount++;
      }
    }
  }

  const rootStyle = getComputedStyle(document.documentElement);
  const tokenNames = [
    '--surface-base',
    '--surface-card',
    '--text-primary',
    '--text-secondary',
    '--accent',
    '--border-hairline',
  ];
  const cssTokens = {};
  for (const name of tokenNames) {
    const val = rootStyle.getPropertyValue(name).trim();
    if (val) cssTokens[name] = val;
  }

  const parseRgb = (s) => {
    const m = /rgba?\((\d+),\s*(\d+),\s*(\d+)/.exec(s || '');
    return m ? [+m[1], +m[2], +m[3]] : null;
  };
  const luma = ([r, g, b]) => {
    const f = (c) => {
      const n = c / 255;
      return n <= 0.03928 ? n / 12.92 : ((n + 0.055) / 1.055) ** 2.4;
    };
    return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
  };

  const lowContrastSamples = [];
  const bgFallback = parseRgb(getComputedStyle(document.body).backgroundColor) || [18, 18, 18];
  for (const el of els.slice(0, 120)) {
    if (!el.childNodes.length || el.children.length > 0) continue;
    const txt = (el.textContent || '').trim();
    if (!txt) continue;
    const cs = getComputedStyle(el);
    const fg = parseRgb(cs.color);
    const bg = parseRgb(cs.backgroundColor);
    const effBg = bg && !cs.backgroundColor.endsWith(', 0)') ? bg : bgFallback;
    if (fg && effBg) {
      const l1 = Math.max(luma(fg), luma(effBg));
      const l2 = Math.min(luma(fg), luma(effBg));
      const ratio = (l1 + 0.05) / (l2 + 0.05);
      if (ratio < 3.0 && lowContrastSamples.length < 5) {
        lowContrastSamples.push({ text: txt.slice(0, 24), ratio: +ratio.toFixed(2) });
      }
    }
  }

  const isRectVisible = (el) => {
    if (!el) return null;
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden' || Number.parseFloat(cs.opacity) === 0) {
      return null;
    }
    const r = el.getBoundingClientRect();
    return r.width > 0 && r.height > 0 ? r : null;
  };
  const toastRect = isRectVisible(document.querySelector('[data-pochita="toast-viewport"]'));
  const bottomRect = isRectVisible(document.querySelector('[data-pochita="app-shell-bottom"]'));
  const toastBottomOverlap = Boolean(
    toastRect &&
      bottomRect &&
      Math.min(toastRect.bottom, bottomRect.bottom) - Math.max(toastRect.top, bottomRect.top) > 0.5,
  );

  let brokenImages = 0;
  for (const img of document.querySelectorAll('img')) {
    if (img.complete && img.naturalWidth === 0) brokenImages++;
  }

  return {
    title: document.title,
    url: location.href,
    viewport: { width: vw, height: vh },
    rootOverflowX,
    horizontalOverflow,
    horizontalOverflowSelectors,
    rootOverflowY,
    toastBottomOverlap,
    brokenImages,
    domNodeCount,
    overflowCount: overflowSamples.length,
    overflowSamples,
    truncatedCount,
    truncated,
    smallTargetsCount,
    smallTargets,
    offscreenCount: offscreenSamples.length,
    offscreenSamples,
    overlapCount,
    cssTokens,
    lowContrastSamples,
  };
})();
