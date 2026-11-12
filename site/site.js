// The player in the hero: the wheel's buttons, a scroll over it and the arrow keys turn its screen.
(function () {
  const screens = [
    ['home-playing', 'Home', "Reborn's Home screen"],
    ['now-playing', 'Now Playing', 'Now Playing with the cover and the progress wave'],
    ['albums', 'Cover Flow', 'Albums as Cover Flow'],
    ['album', 'An album', 'An album page with its songs'],
    ['search-results', 'Search', 'Search results'],
    ['radio-playing', 'Internet radio', 'A radio station playing'],
    ['now-playing-book', 'An audiobook', 'An audiobook playing'],
    ['quick-menu', 'Quick menu', 'The quick menu'],
    ['settings', 'Settings', 'Settings'],
  ];
  const image = document.getElementById('screen');
  const caption = document.getElementById('caption');
  const player = document.getElementById('player');
  let at = 0;
  let touched = false;

  function swap(target, source, alt) {
    const next = new Image();
    next.onload = function () {
      target.classList.add('out');
      setTimeout(function () {
        target.src = source;
        target.alt = alt;
        target.classList.remove('out');
      }, 150);
    };
    next.src = source;
  }
  function show(step) {
    at = (at + step + screens.length) % screens.length;
    swap(image, 'img/' + screens[at][0] + '.png', screens[at][2]);
    caption.textContent = screens[at][1];
  }
  function turn(step) { touched = true; show(step); }

  player.querySelector('.w-prev').addEventListener('click', function () { turn(-1); });
  player.querySelector('.w-next').addEventListener('click', function () { turn(1); });
  player.querySelector('.w-ok').addEventListener('click', function () { turn(1); });
  player.addEventListener('keydown', function (event) {
    if (event.key === 'ArrowLeft') turn(-1);
    if (event.key === 'ArrowRight') turn(1);
  });
  let last = 0;
  player.querySelector('.wheel').addEventListener('wheel', function (event) {
    event.preventDefault();
    const now = Date.now();
    if (now - last < 220) return;
    last = now;
    turn(event.deltaY > 0 ? 1 : -1);
  }, { passive: false });

  const still = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  if (!still) setInterval(function () { if (!touched && !document.hidden) show(1); }, 3800);

  // Skins: one picture, a chip for each.
  const skins = [
    ['classic', 'Classic'], ['nano-glass', 'Nano Glass'], ['walkman', 'Walkman'], ['terminal', 'Terminal'],
    ['winamp', 'Winamp'], ['paper', 'Paper'], ['big-type', 'Big Type'], ['cassette', 'Cassette'],
    ['neon', 'Neon Night'], ['zune', 'Zune'], ['bunny-bow', 'Bunny Bow'], ['luna-xp', 'Luna XP'],
    ['mini-blue', 'Mini Blue'], ['mini-gold', 'Mini Gold'], ['mini-green', 'Mini Green'],
    ['mini-pink', 'Mini Pink'], ['mini-silver', 'Mini Silver'],
  ];
  const skin = document.getElementById('skin');
  const chips = document.getElementById('chips');
  skins.forEach(function (entry, index) {
    const chip = document.createElement('button');
    chip.type = 'button';
    chip.textContent = entry[1];
    chip.setAttribute('aria-pressed', index === 0 ? 'true' : 'false');
    chip.addEventListener('click', function () {
      chips.querySelectorAll('button').forEach(function (other) { other.setAttribute('aria-pressed', 'false'); });
      chip.setAttribute('aria-pressed', 'true');
      swap(skin, 'img/skin-' + entry[0] + '.png', 'Now Playing in the ' + entry[1] + ' skin');
    });
    chips.appendChild(chip);
  });
})();
