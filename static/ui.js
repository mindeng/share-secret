// 给一个密码输入框旁边加一个"显示/隐藏"切换按钮（眼睛图标）。
// input 处于 password 时按钮显示睁眼图标（aria-label「显示密码」），点击切到明文并显示闭眼图标。

// 静态、可信的 SVG 字符串；用 DOMParser 解析成节点，避免使用 innerHTML。
const _EYE_OPEN_SVG =
    '<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/></svg>';
const _EYE_OFF_SVG =
    '<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24"/><line x1="1" y1="1" x2="23" y2="23"/></svg>';

function _svgIcon(markup) {
    const doc = new DOMParser().parseFromString(markup, 'image/svg+xml');
    return document.importNode(doc.documentElement, true);
}

function attachPasswordToggle(input) {
    if (!input) return;

    // 用 flex 容器包裹 input，让输入框与按钮并排
    const wrap = document.createElement('div');
    wrap.style.cssText = 'display:flex; gap:0.5rem; align-items:stretch;';
    input.parentNode.insertBefore(wrap, input);
    wrap.appendChild(input);
    input.style.flex = '1';
    input.style.marginTop = '0';

    const btn = document.createElement('button');
    btn.type = 'button';
    btn.style.cssText =
        'width:auto; padding:0.4rem 0.8rem; display:inline-flex; align-items:center;';
    btn.setAttribute('aria-label', '显示密码');
    btn.replaceChildren(_svgIcon(_EYE_OPEN_SVG));

    btn.addEventListener('click', () => {
        const show = input.type === 'password';
        input.type = show ? 'text' : 'password';
        btn.replaceChildren(_svgIcon(show ? _EYE_OFF_SVG : _EYE_OPEN_SVG));
        btn.setAttribute('aria-label', show ? '隐藏密码' : '显示密码');
    });

    wrap.appendChild(btn);
}
