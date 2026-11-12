# Reborn's website

The page at <https://schulzcode.github.io/Y2Reborn/>. Plain HTML, one stylesheet and one script; nothing to
build. `.github/workflows/pages.yml` publishes this folder when it changes on `main`.

- Colours and type are the base skin's (`crates/reborn-ui/src/theme.rs`); the font is Open Sans, a Latin subset
  made with `pyftsubset` (`fonts/OpenSans-OFL.txt`).
- `img/` holds 480×360 pictures from `reborn-preview` (`tools/preview/render_previews.py`), `img/skin-*.png` is
  Now Playing (`14-now-playing`) rendered with `--skin skins/<name>`. `img/videos.png` and `img/film.png` are
  captures from a player. Render them again when the UI changes visibly.
- To look at it locally, open `index.html` in a browser.
