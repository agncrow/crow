// A card's stars: five buttons, the first `props.stars` filled; pressing one
// sends its count to the handler the page names for `rate`.
export function mount(el, props, send) {
  for (let i = 1; i <= 5; i++) {
    const b = document.createElement('button');
    b.className = 'star';
    b.dataset.n = String(i);
    b.onclick = () => send('rate', i);
    el.appendChild(b);
  }
  update(el, props);
}

export function update(el, props) {
  for (const b of el.querySelectorAll('.star')) {
    b.textContent = Number(b.dataset.n) <= props.stars ? '★' : '☆';
  }
}
