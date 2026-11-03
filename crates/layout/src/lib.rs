//! Layout engine: strip vertikal dengan dua lajur. Murni logika, tanpa Win32.
//!
//! Tiap window punya lebar (`Span`: penuh, lajur kiri, lajur kanan) dan tinggi
//! sendiri (1/3, 1/2, penuh). Window disusun berurutan seperti aliran teks:
//!  - window `Full` ditaruh di bawah kedua lajur,
//!  - window `Left` / `Right` ditumpuk di lajur masing-masing.
//! Tinggi tiap window independen. Viewport meng-scroll mengikuti fokus.
//!
//! Aksi: snap (tinggi / sisi), maximize, focus, dan move_window (tukar posisi
//! dengan tetangga di arah tertentu, apa pun ukurannya).

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Height {
    Third,
    Half,
    Full,
}

impl Height {
    fn grow(self) -> Self {
        match self {
            Height::Third => Height::Half,
            _ => Height::Full,
        }
    }
    fn shrink(self) -> Self {
        match self {
            Height::Full => Height::Half,
            _ => Height::Third,
        }
    }
    fn pixels(self, viewport_h: i32) -> i32 {
        match self {
            Height::Third => viewport_h / 3,
            Height::Half => viewport_h / 2,
            Height::Full => viewport_h,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Span {
    Full,
    Left,
    Right,
    /// Tersembunyi (di-minimize, cloaked, dll): tidak ikut layout, tapi slotnya
    /// (urutan, lebar `restore`, tinggi) diingat sampai muncul lagi.
    Hidden,
}

impl Span {
    fn opposite(self) -> Span {
        match self {
            Span::Left => Span::Right,
            Span::Right => Span::Left,
            Span::Full => Span::Full,
            Span::Hidden => Span::Hidden,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Item {
    pub id: WindowId,
    pub span: Span,
    pub height: Height,
    /// Lebar yang dipulihkan saat window muncul lagi (dipakai kalau `span` = Hidden).
    pub restore: Span,
}

/// Hasil layout untuk satu window. `visible = false` berarti seluruhnya
/// di luar viewport (platform layer bebas menyembunyikan / memarkir window itu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub id: WindowId,
    pub rect: Rect,
    pub visible: bool,
}

/// Satu window di peta overview (koordinat strip, tanpa scroll).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverviewItem {
    pub id: WindowId,
    pub rect: Rect,
}

/// Peta seluruh strip untuk tampilan overview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overview {
    pub items: Vec<OverviewItem>,
    /// Lebar dan tinggi total strip.
    pub width: i32,
    pub height: i32,
    /// Bagian strip yang sedang tampil di layar: mulai dari `view_y`, setinggi `view_h`.
    pub view_y: i32,
    pub view_h: i32,
}

/// Posisi window di koordinat strip (belum dikurangi scroll).
#[derive(Debug, Clone, Copy)]
struct Flow {
    id: WindowId,
    span: Span,
    top: i32,
    h: i32,
}

impl Flow {
    fn bottom(&self) -> i32 {
        self.top + self.h
    }
    fn x_range(&self, w: i32) -> (i32, i32) {
        match self.span {
            Span::Full => (0, w),
            Span::Left => (0, w / 2),
            Span::Right => (w / 2, w),
            Span::Hidden => (0, 0),
        }
    }
}

pub struct Strip {
    items: Vec<Item>,
    focused: Option<WindowId>,
    scroll_y: i32,
    viewport: Rect,
    /// Tinggi minimum yang diukur dari window asli (app menolak dikecilkan).
    min_heights: HashMap<WindowId, i32>,
    /// Jarak antar window dan ke tepi layar (piksel).
    gap: i32,
}

impl Strip {
    pub fn new(viewport: Rect) -> Self {
        Self {
            items: Vec::new(),
            focused: None,
            scroll_y: 0,
            viewport,
            min_heights: HashMap::new(),
            gap: 0,
        }
    }

    /// Atur gap. Antar window dan ke tepi layar sama besar.
    pub fn set_gap(&mut self, gap: i32) {
        self.gap = gap.max(0);
        self.ensure_visible();
    }

    /// Area efektif: work area dikurangi setengah gap di tiap sisi. Tiap window
    /// lalu dikecilkan setengah gap lagi, jadi jarak antar window = gap penuh.
    fn content(&self) -> Rect {
        let g = self.gap / 2;
        Rect {
            x: self.viewport.x + g,
            y: self.viewport.y + g,
            w: (self.viewport.w - 2 * g).max(1),
            h: (self.viewport.h - 2 * g).max(1),
        }
    }

    pub fn focused(&self) -> Option<WindowId> {
        self.focused
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn set_viewport(&mut self, viewport: Rect) {
        self.viewport = viewport;
        self.ensure_visible();
    }

    /// Catat tinggi minimum window. True kalau nilainya naik (layout berubah).
    pub fn set_min_height(&mut self, id: WindowId, h: i32) -> bool {
        let h = h.min((self.content().h - 2 * (self.gap / 2)).max(0));
        let cur = self.min_heights.get(&id).copied().unwrap_or(0);
        if h > cur {
            self.min_heights.insert(id, h);
            self.ensure_visible();
            true
        } else {
            false
        }
    }

    fn index_of(&self, id: WindowId) -> Option<usize> {
        self.items.iter().position(|it| it.id == id)
    }

    /// Tinggi window: target atau minimum aslinya, mana yang lebih besar.
    fn item_px(&self, it: &Item) -> i32 {
        // tinggi minimum adalah tinggi window; slot perlu ditambah gap
        let min = self
            .min_heights
            .get(&it.id)
            .map_or(0, |m| m + 2 * (self.gap / 2));
        it.height.pixels(self.content().h).max(min)
    }

    /// Susun semua window di koordinat strip.
    ///
    /// Window penuh lebar memisahkan strip jadi "run" (blok berisi dua lajur).
    /// Di tiap run, lihat batas-batas NOMINAL (tinggi target, tanpa tinggi
    /// minimum) di kedua lajur: kalau sebuah batas seharusnya sejajar tapi jadi
    /// tidak rata karena tinggi minimum sebuah app, window di lajur yang lebih
    /// pendek diregangkan supaya batas itu rata. Dengan begitu susunan seperti
    /// 2x2 kuarter tetap sejajar baris demi baris, dan gap seragam.
    fn flow(&self) -> Vec<Flow> {
        let content_h = self.content().h;
        let dummy = Flow { id: WindowId(0), span: Span::Full, top: 0, h: 0 };
        let mut out = vec![dummy; self.items.len()];
        let mut base = 0i32;
        let (mut left, mut right): (Vec<usize>, Vec<usize>) = (Vec::new(), Vec::new());
        for (i, it) in self.items.iter().enumerate() {
            match it.span {
                Span::Left => left.push(i),
                Span::Right => right.push(i),
                Span::Hidden => {
                    out[i] = Flow { id: it.id, span: Span::Hidden, top: base, h: 0 };
                }
                Span::Full => {
                    let top = self.place_run(&mut out, base, &left, &right, content_h);
                    left.clear();
                    right.clear();
                    let h = self.item_px(it);
                    out[i] = Flow { id: it.id, span: Span::Full, top, h };
                    base = top + h;
                }
            }
        }
        self.place_run(&mut out, base, &left, &right, content_h);
        out
    }

    /// Tata satu run mulai dari `base`. Mengembalikan dasar run (yang terendah).
    fn place_run(
        &self,
        out: &mut [Flow],
        base: i32,
        left: &[usize],
        right: &[usize],
        content_h: i32,
    ) -> i32 {
        let mut hl: Vec<i32> = left.iter().map(|&k| self.item_px(&self.items[k])).collect();
        let mut hr: Vec<i32> = right.iter().map(|&k| self.item_px(&self.items[k])).collect();

        let nominal = |k: usize| self.items[k].height.pixels(content_h);
        let cum = |idxs: &[usize]| -> Vec<i32> {
            let mut c = 0;
            idxs.iter()
                .map(|&k| {
                    c += nominal(k);
                    c
                })
                .collect()
        };
        let (cl, cr) = (cum(left), cum(right));

        let (mut i, mut j) = (0, 0);
        while i < cl.len() && j < cr.len() {
            let d = cl[i] - cr[j];
            if d.abs() <= 3 {
                let al: i32 = hl[..=i].iter().sum();
                let ar: i32 = hr[..=j].iter().sum();
                if al < ar {
                    hl[i] += ar - al;
                } else if ar < al {
                    hr[j] += al - ar;
                }
                i += 1;
                j += 1;
            } else if d < 0 {
                i += 1;
            } else {
                j += 1;
            }
        }

        let mut yl = base;
        for (n, &k) in left.iter().enumerate() {
            out[k] = Flow { id: self.items[k].id, span: Span::Left, top: yl, h: hl[n] };
            yl += hl[n];
        }
        let mut yr = base;
        for (n, &k) in right.iter().enumerate() {
            out[k] = Flow { id: self.items[k].id, span: Span::Right, top: yr, h: hr[n] };
            yr += hr[n];
        }
        yl.max(yr)
    }

    /// Window baru: penuh lebar & tinggi, tepat setelah window yang fokus.
    pub fn add_window(&mut self, id: WindowId) {
        if self.index_of(id).is_some() {
            return;
        }
        // Di bawah SELURUH blok window yang fokus (bukan tepat setelahnya), supaya
        // window di lajur sebelah tidak ikut terdorong ke bawah window baru.
        let at = self
            .focused
            .and_then(|f| self.index_of(f))
            .map_or(self.items.len(), |f| {
                if self.items[f].span == Span::Full {
                    f + 1
                } else {
                    (f + 1..self.items.len())
                        .find(|&k| self.items[k].span == Span::Full)
                        .unwrap_or(self.items.len())
                }
            });
        self.items.insert(
            at,
            Item { id, span: Span::Full, height: Height::Full, restore: Span::Full },
        );
        self.focused = Some(id);
        self.ensure_visible();
    }

    pub fn remove_window(&mut self, id: WindowId) {
        let Some(idx) = self.index_of(id) else { return };
        self.items.remove(idx);
        self.min_heights.remove(&id);
        if self.focused == Some(id) {
            self.focused = self.pick_focus(idx);
        }
        self.ensure_visible();
    }

    /// Window terlihat terdekat dari posisi `idx` (maju dulu, lalu mundur).
    fn pick_focus(&self, idx: usize) -> Option<WindowId> {
        (idx..self.items.len())
            .find(|&k| self.items[k].span != Span::Hidden)
            .or_else(|| (0..idx.min(self.items.len())).rev().find(|&k| self.items[k].span != Span::Hidden))
            .map(|k| self.items[k].id)
    }

    /// Sembunyikan / tampilkan window tanpa kehilangan slotnya. Dipakai saat
    /// window di-minimize atau muncul lagi. True kalau ada perubahan.
    pub fn set_hidden(&mut self, id: WindowId, hide: bool) -> bool {
        let Some(i) = self.index_of(id) else { return false };
        if hide {
            if self.items[i].span == Span::Hidden {
                return false;
            }
            self.items[i].restore = self.items[i].span;
            self.items[i].span = Span::Hidden;
            if self.focused == Some(id) {
                self.focused = self.pick_focus(i);
            }
        } else {
            if self.items[i].span != Span::Hidden {
                return false;
            }
            self.items[i].span = self.items[i].restore;
            if self.focused.is_none() {
                self.focused = Some(id);
            }
        }
        self.ensure_visible();
        true
    }

    pub fn set_focus(&mut self, id: WindowId) {
        if self.index_of(id).map_or(false, |i| self.items[i].span != Span::Hidden) {
            self.focused = Some(id);
            self.ensure_visible();
        }
    }

    /// Snap window yang fokus.
    /// Up: tinggi makin pendek, Down: makin tinggi (per window).
    /// Left/Right: pindah ke lajur itu (lihat `snap_side`).
    pub fn snap(&mut self, dir: Dir) {
        let Some(id) = self.focused else { return };
        let Some(i) = self.index_of(id) else { return };
        match dir {
            // tinggi diubah user -> ukur ulang minimum window ini dari nol
            Dir::Up => {
                self.min_heights.remove(&id);
                self.items[i].height = self.items[i].height.shrink();
            }
            Dir::Down => {
                self.min_heights.remove(&id);
                self.items[i].height = self.items[i].height.grow();
            }
            Dir::Left => self.snap_side(i, Span::Left),
            Dir::Right => self.snap_side(i, Span::Right),
        }
        self.ensure_visible();
    }

    /// Jadikan window `i` penuh lebar. Window di sampingnya (yang mulai di atas
    /// dasar window ini) tetap di atas; window ini turun di bawahnya.
    fn make_full(&mut self, i: usize) {
        let flow = self.flow();
        let w_bottom = flow[i].bottom();
        let mut w = self.items[i];
        w.span = Span::Full;
        let mut before = Vec::new();
        let mut after = Vec::new();
        for (j, it) in self.items.iter().enumerate() {
            if j == i {
                continue;
            }
            if flow[j].top < w_bottom {
                before.push(*it);
            } else {
                after.push(*it);
            }
        }
        before.push(w);
        before.extend(after);
        self.items = before;
    }

    /// Left/Right pada window `i`.
    ///
    ///  - sudah di sisi itu            -> tidak ada yang terjadi
    ///  - di lajur seberang            -> jadi penuh lebar dulu (tinggi tetap);
    ///                                    tekan lagi untuk pindah ke sisi itu
    ///  - penuh lebar                  -> masuk lajur, dengan aturan penghuni (X)
    ///    yang terdekat di sekitarnya:
    ///      * seberang X kosong        -> SWAP: X pindah ke seberang, window ambil posisi X
    ///      * seberang X berisi window yang tidak lebih tinggi dari X
    ///        (baris kuarter sudah penuh) -> window masuk DI BAWAH X
    ///      * seberang X lebih tinggi dari X (mis. setengah layar)
    ///                                 -> window di ATAS X, X tergeser ke bawah
    ///      * tidak ada X              -> cukup pindah lajur
    fn snap_side(&mut self, i: usize, target: Span) {
        let current = self.items[i].span;
        if current == target {
            return;
        }
        let opposite = target.opposite();
        if current == opposite {
            self.make_full(i);
            return;
        }

        let flow = self.flow();
        let run_start = self.items[..i]
            .iter()
            .rposition(|it| it.span == Span::Full)
            .map_or(0, |p| p + 1);
        let run_end = self.items[i + 1..]
            .iter()
            .position(|it| it.span == Span::Full)
            .map_or(self.items.len(), |p| i + 1 + p);

        let x = (run_start..i)
            .rev()
            .find(|&j| self.items[j].span == target)
            .or_else(|| (i + 1..run_end).find(|&j| self.items[j].span == target));

        let Some(j) = x else {
            self.items[i].span = target;
            return;
        };

        let (tj, bj) = (flow[j].top, flow[j].bottom());
        let beside: Vec<Flow> = flow
            .iter()
            .enumerate()
            .filter(|(k, f)| {
                *k != i && *k != j && f.span == opposite && f.top < bj && f.bottom() > tj
            })
            .map(|(_, f)| *f)
            .collect();

        let above = if beside.is_empty() {
            self.items[j].span = opposite; // swap
            true
        } else {
            // baris kuarter penuh -> di bawah X; ada yang lebih tinggi -> di atas X
            !beside.iter().all(|f| f.top >= tj && f.bottom() <= bj)
        };

        self.items[i].span = target;
        let w = self.items.remove(i);
        let jj = if j > i { j - 1 } else { j };
        self.items.insert(if above { jj } else { jj + 1 }, w);
    }

    /// Kembalikan window yang fokus jadi penuh (lebar + tinggi), di bawah
    /// window-window yang ada di sampingnya.
    pub fn maximize(&mut self) {
        let Some(id) = self.focused else { return };
        let Some(i) = self.index_of(id) else { return };
        self.make_full(i);
        if let Some(k) = self.index_of(id) {
            self.items[k].height = Height::Full;
        }
        self.ensure_visible();
    }

    /// Window tetangga terdekat di arah `dir`, berdasarkan posisi di layar.
    fn neighbor(&self, dir: Dir) -> Option<WindowId> {
        self.neighbor_of(self.focused?, dir)
    }

    /// Tetangga terdekat dari `id` di arah `dir`, tanpa mengubah fokus.
    pub fn neighbor_of(&self, id: WindowId, dir: Dir) -> Option<WindowId> {
        let flow = self.flow();
        let w = self.content().w;
        let f = flow.iter().find(|c| c.id == id)?;
        let (fx0, fx1) = f.x_range(w);
        let (fy0, fy1) = (f.top, f.bottom());

        let mut best: Option<((bool, i32, i32, i32), WindowId)> = None;
        for c in flow.iter().filter(|c| c.id != id && c.span != Span::Hidden) {
            let (cx0, cx1) = c.x_range(w);
            let (cy0, cy1) = (c.top, c.bottom());
            let h_ov = (fx1.min(cx1) - fx0.max(cx0)).max(0);
            let v_ov = (fy1.min(cy1) - fy0.max(cy0)).max(0);
            let key = match dir {
                Dir::Up if cy1 <= fy0 => Some((h_ov == 0, fy0 - cy1, -h_ov, c.top)),
                Dir::Down if cy0 >= fy1 => Some((h_ov == 0, cy0 - fy1, -h_ov, c.top)),
                Dir::Left if cx1 <= fx0 && v_ov > 0 => Some((false, fx0 - cx1, -v_ov, c.top)),
                Dir::Right if cx0 >= fx1 && v_ov > 0 => Some((false, cx0 - fx1, -v_ov, c.top)),
                _ => None,
            };
            if let Some(k) = key {
                if best.map_or(true, |(bk, _)| k < bk) {
                    best = Some((k, c.id));
                }
            }
        }
        best.map(|(_, n)| n)
    }

    /// Pindahkan fokus berdasarkan posisi sebenarnya di layar.
    pub fn focus(&mut self, dir: Dir) {
        if let Some(n) = self.neighbor(dir) {
            self.focused = Some(n);
            self.ensure_visible();
        }
    }

    /// Tukar POSISI window yang fokus dengan tetangga di arah `dir`; ukuran
    /// (lebar/tinggi) tetap milik masing-masing window.
    ///  - satu lajur, atau salah satunya penuh lebar -> tukar urutan
    ///  - beda lajur                                  -> tukar sisi
    pub fn move_window(&mut self, dir: Dir) {
        let Some(id) = self.focused else { return };
        let Some(n) = self.neighbor(dir) else { return };
        let (Some(a), Some(b)) = (self.index_of(id), self.index_of(n)) else { return };
        let (sa, sb) = (self.items[a].span, self.items[b].span);
        if sa != Span::Full && sb != Span::Full && sa != sb {
            self.items[a].span = sb;
            self.items[b].span = sa;
        } else {
            self.items.swap(a, b);
        }
        self.ensure_visible();
    }

    fn ensure_visible(&mut self) {
        let flow = self.flow();
        let vh = self.content().h;
        let content = flow.iter().map(|f| f.bottom()).max().unwrap_or(0);
        self.scroll_y = self.scroll_y.min((content - vh).max(0));

        let Some(f) = self
            .focused
            .and_then(|id| flow.iter().find(|f| f.id == id))
        else {
            self.scroll_y = 0;
            return;
        };
        let (top, bottom) = (f.top, f.bottom());
        if top < self.scroll_y {
            self.scroll_y = top;
        } else if bottom > self.scroll_y + vh {
            self.scroll_y = bottom - vh;
        }
        self.scroll_y = self.scroll_y.min(top).max(0);
    }

    /// Window berikutnya (atau sebelumnya) menurut urutan baca: atas ke bawah,
    /// kiri ke kanan, berputar. Dipakai untuk Tab di overview. None kalau hanya
    /// ada satu window atau `id` tidak dikenal / tersembunyi.
    pub fn cycle_from(&self, id: WindowId, forward: bool) -> Option<WindowId> {
        let c = self.content();
        let mut order: Vec<(i32, i32, WindowId)> = self
            .flow()
            .iter()
            .filter(|f| f.span != Span::Hidden)
            .map(|f| (f.top, f.x_range(c.w).0, f.id))
            .collect();
        order.sort_by_key(|&(top, x, _)| (top, x));
        let n = order.len();
        let pos = order.iter().position(|&(_, _, w)| w == id)?;
        if n < 2 {
            return None;
        }
        let next = if forward { (pos + 1) % n } else { (pos + n - 1) % n };
        Some(order[next].2)
    }

    /// Peta seluruh strip (termasuk bagian di luar layar) untuk overview.
    pub fn overview(&self) -> Overview {
        let c = self.content();
        let items: Vec<OverviewItem> = self
            .flow()
            .iter()
            .filter(|f| f.span != Span::Hidden)
            .map(|f| {
                let (x0, x1) = f.x_range(c.w);
                OverviewItem { id: f.id, rect: Rect { x: x0, y: f.top, w: x1 - x0, h: f.h } }
            })
            .collect();
        let height = items
            .iter()
            .map(|i| i.rect.y + i.rect.h)
            .max()
            .unwrap_or(0)
            .max(c.h);
        Overview { items, width: c.w, height, view_y: self.scroll_y, view_h: c.h }
    }

    pub fn layout(&self) -> Vec<Placement> {
        let vp = self.viewport;
        let c = self.content();
        let g = self.gap / 2;
        let half = c.w / 2;
        self.flow()
            .into_iter()
            .filter(|f| f.span != Span::Hidden)
            .map(|f| {
                let top = c.y + f.top - self.scroll_y;
                let (x, w) = match f.span {
                    Span::Full | Span::Hidden => (c.x, c.w),
                    Span::Left => (c.x, half),
                    Span::Right => (c.x + half, c.w - half),
                };
                Placement {
                    id: f.id,
                    rect: Rect {
                        x: x + g,
                        y: top + g,
                        w: (w - 2 * g).max(1),
                        h: (f.h - 2 * g).max(1),
                    },
                    visible: top + f.h > vp.y && top < vp.y + vp.h,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: WindowId = WindowId(1);
    const B: WindowId = WindowId(2);
    const C: WindowId = WindowId(3);
    const X: WindowId = WindowId(4);
    const D: WindowId = WindowId(5);

    fn strip() -> Strip {
        Strip::new(Rect { x: 0, y: 0, w: 1920, h: 1080 })
    }

    fn r(s: &Strip, id: WindowId) -> Rect {
        s.layout().into_iter().find(|p| p.id == id).unwrap().rect
    }

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn new_window_goes_below_and_scrolls() {
        let mut s = strip();
        s.add_window(A);
        s.add_window(B);
        assert_eq!(s.focused(), Some(B));
        assert_eq!(r(&s, B).y, 0);
        assert_eq!(r(&s, A).y, -1080);
        let a = s.layout().into_iter().find(|p| p.id == A).unwrap();
        assert!(!a.visible);
    }

    #[test]
    fn up_shrinks_and_down_grows() {
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Up);
        assert_eq!(r(&s, A).h, 540);
        s.snap(Dir::Up);
        assert_eq!(r(&s, A).h, 360);
        s.snap(Dir::Down);
        s.snap(Dir::Down);
        assert_eq!(r(&s, A).h, 1080);
    }

    #[test]
    fn snap_left_gives_half_width_and_leaves_right_free() {
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        assert_eq!(r(&s, A), rect(0, 0, 960, 540));
    }

    #[test]
    fn snapping_same_side_again_is_noop() {
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Left);
        let before = s.items().to_vec();
        s.snap(Dir::Left);
        assert_eq!(s.items(), &before[..]);
    }

    #[test]
    fn opposite_side_goes_full_width_first_then_other_side() {
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.snap(Dir::Right); // lebar penuh dulu, tinggi tetap
        assert_eq!(r(&s, A), rect(0, 0, 1920, 540));
        s.snap(Dir::Right); // baru pindah ke kanan
        assert_eq!(r(&s, A), rect(960, 0, 960, 540));
    }

    #[test]
    fn heights_are_independent_per_window() {
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.add_window(B);
        s.snap(Dir::Right); // B tetap setinggi penuh
        assert_eq!(r(&s, A), rect(0, 0, 960, 540));
        assert_eq!(r(&s, B), rect(960, 0, 960, 1080));
    }

    #[test]
    fn swap_when_opposite_lane_is_free() {
        // A kuarter kiri atas, kanan kosong. B snap kiri -> B kiri atas, A kanan atas.
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.add_window(B);
        s.snap(Dir::Left);
        assert_eq!(r(&s, B), rect(0, 0, 960, 1080));
        assert_eq!(r(&s, A), rect(960, 0, 960, 540));
    }

    /// Y: separuh layar kanan (penuh tinggi). X: kuarter kiri atas.
    fn y_right_x_top_left() -> Strip {
        let mut s = strip();
        s.add_window(A); // A = Y
        s.snap(Dir::Right);
        s.add_window(X);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s
    }

    #[test]
    fn push_down_when_opposite_lane_is_occupied() {
        let mut s = y_right_x_top_left();
        assert_eq!(r(&s, X), rect(0, 0, 960, 540));
        s.add_window(B);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        // B di kiri atas, X turun ke bawahnya, A (kanan penuh) tidak tergeser
        assert_eq!(r(&s, B), rect(0, 0, 960, 540));
        assert_eq!(r(&s, X), rect(0, 540, 960, 540));
        assert_eq!(r(&s, A), rect(960, 0, 960, 1080));
    }

    #[test]
    fn maximize_restores_full_size() {
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.maximize();
        assert_eq!(r(&s, A), rect(0, 0, 1920, 1080));
    }

    #[test]
    fn min_height_stretches_aligned_neighbor_to_keep_edges_even() {
        // A dan B sama-sama setengah tinggi, berdampingan. B punya minimum 600:
        // A diregangkan ke 600 supaya dasar keduanya rata.
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.add_window(B);
        s.snap(Dir::Up);
        s.snap(Dir::Right);
        assert!(s.set_min_height(B, 600));
        assert_eq!(r(&s, B).h, 600);
        assert_eq!(r(&s, A).h, 600);
        assert!(!s.set_min_height(B, 500));
    }

    #[test]
    fn quarter_grid_rows_stay_aligned_with_min_height() {
        // 2x2: A B / C D. B punya tinggi minimum -> kedua baris tetap sejajar.
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.add_window(B);
        s.snap(Dir::Up);
        s.snap(Dir::Right);
        s.add_window(C);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.add_window(D);
        s.snap(Dir::Up);
        s.snap(Dir::Right);
        s.set_min_height(B, 600);
        let (a, b, c, d) = (r(&s, A), r(&s, B), r(&s, C), r(&s, D));
        assert_eq!(a.y, b.y);
        assert_eq!(a.h, b.h);
        assert_eq!(c.y, d.y);
        assert_eq!(c.y - a.y, 600);
        assert_eq!(c.y + c.h, d.y + d.h);
    }

    #[test]
    fn new_window_goes_below_whole_block_without_pushing_neighbors() {
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.add_window(B);
        s.snap(Dir::Up);
        s.snap(Dir::Right);
        s.set_focus(A);
        s.add_window(C);
        assert_eq!(r(&s, B).y, r(&s, A).y); // B tidak turun
        assert_eq!(r(&s, C).y - r(&s, A).y, 540);
    }

    #[test]
    fn min_height_pushes_window_below_in_same_lane() {
        let mut s = y_right_x_top_left();
        s.add_window(B);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.set_focus(B);
        s.set_min_height(B, 700);
        assert_eq!(r(&s, B).h, 700);
        assert_eq!(r(&s, X).y, r(&s, B).y + 700);
    }

    #[test]
    fn min_height_is_capped_to_viewport() {
        let mut s = strip();
        s.add_window(A);
        s.set_min_height(A, 5000);
        assert_eq!(r(&s, A).h, 1080);
    }

    #[test]
    fn focus_follows_screen_geometry() {
        let mut s = y_right_x_top_left();
        s.add_window(B);
        s.snap(Dir::Up);
        s.snap(Dir::Left); // kiri: B (atas), X (bawah). kanan: A
        assert_eq!(s.focused(), Some(B));
        s.focus(Dir::Down);
        assert_eq!(s.focused(), Some(X));
        s.focus(Dir::Up);
        assert_eq!(s.focused(), Some(B));
        s.focus(Dir::Right);
        assert_eq!(s.focused(), Some(A));
        s.focus(Dir::Left);
        assert_eq!(s.focused(), Some(B));
    }

    #[test]
    fn remove_moves_focus_and_clamps_scroll() {
        let mut s = strip();
        s.add_window(A);
        s.add_window(B);
        s.add_window(C);
        s.remove_window(C);
        assert_eq!(s.items().len(), 2);
        assert_eq!(s.focused(), Some(B));
        assert_eq!(r(&s, B).y, 0);
    }

    #[test]
    fn vertical_focus_scrolls_back_up() {
        let mut s = strip();
        s.add_window(A);
        s.add_window(B);
        s.focus(Dir::Up);
        assert_eq!(s.focused(), Some(A));
        assert_eq!(r(&s, A).y, 0);
    }

    #[test]
    fn full_width_goes_below_window_beside_it() {
        // A kiri atas, B kanan atas. A lebar penuh -> turun di bawah B.
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.add_window(B);
        s.snap(Dir::Up);
        s.snap(Dir::Right);
        s.set_focus(A);
        s.snap(Dir::Right);
        assert_eq!(r(&s, A), rect(0, 540, 1920, 540));
        assert_eq!(r(&s, B), rect(960, 0, 960, 540));
    }

    #[test]
    fn goes_below_when_quarter_row_is_complete() {
        // A kiri atas, B kanan atas (baris kuarter penuh). C kuarter kiri -> di bawah A.
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.add_window(B);
        s.snap(Dir::Up);
        s.snap(Dir::Right);
        s.add_window(C);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        assert_eq!(r(&s, A), rect(0, 0, 960, 540));
        assert_eq!(r(&s, B), rect(960, 0, 960, 540));
        assert_eq!(r(&s, C), rect(0, 540, 960, 540));
    }

    #[test]
    fn maximize_goes_below_neighbors_without_dragging_them() {
        // kiri: B (atas), A (bawah). kanan: C setengah layar penuh tinggi.
        let mut s = strip();
        s.add_window(C);
        s.snap(Dir::Right);
        s.add_window(A);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.add_window(B);
        s.snap(Dir::Up);
        s.snap(Dir::Left); // B di atas A
        s.set_focus(A);
        s.maximize();
        assert_eq!(r(&s, A).w, 1920);
        assert_eq!(r(&s, A).h, 1080);
        assert_eq!(r(&s, A).y - r(&s, C).y, 1080); // di bawah A dan C
        assert_eq!(r(&s, C).y, r(&s, B).y); // C tidak ikut turun
    }

    #[test]
    fn move_window_with_full_width_neighbor_keeps_sizes() {
        // A penuh lebar setengah tinggi; di bawahnya B kiri, C kanan.
        // B naik: B tetap kuarter, A tetap penuh lebar, C ikut turun di bawah A.
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Up);
        s.add_window(B);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.add_window(C);
        s.snap(Dir::Up);
        s.snap(Dir::Right);
        s.set_focus(B);
        s.move_window(Dir::Up);
        assert_eq!(s.focused(), Some(B));
        assert_eq!(r(&s, B), rect(0, 0, 960, 540));
        assert_eq!(r(&s, A), rect(0, 540, 1920, 540));
        assert_eq!(r(&s, C).y, 1080);
    }

    #[test]
    fn move_window_in_same_lane_swaps_positions_not_sizes() {
        // kiri: W (tinggi penuh) di atas, X (setengah) di bawah. Kanan: Y.
        let mut s = strip();
        s.add_window(A); // A = Y
        s.snap(Dir::Right);
        s.add_window(X);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.add_window(B); // B = W
        s.snap(Dir::Left); // di atas X, tinggi penuh
        s.set_focus(X);
        let (wh, xh) = (r(&s, B).h, r(&s, X).h);
        assert_eq!((wh, xh), (1080, 540));
        s.move_window(Dir::Up);
        assert_eq!(r(&s, X).h, 540);
        assert_eq!(r(&s, B).h, 1080);
        assert_eq!(r(&s, B).y - r(&s, X).y, 540); // X sekarang di atas B
        assert_eq!(r(&s, X).y, r(&s, A).y);
    }

    #[test]
    fn move_window_across_lanes_swaps_sides_not_heights() {
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.add_window(B);
        s.snap(Dir::Up);
        s.snap(Dir::Up);
        s.snap(Dir::Right); // B sepertiga di kanan
        s.set_focus(A);
        s.move_window(Dir::Right);
        assert_eq!(r(&s, A), rect(960, 0, 960, 540));
        assert_eq!(r(&s, B), rect(0, 0, 960, 360));
    }

    #[test]
    fn changing_height_forgets_measured_min_height() {
        let mut s = strip();
        s.add_window(A);
        s.set_min_height(A, 600);
        assert_eq!(r(&s, A).h, 1080);
        s.snap(Dir::Up);
        assert_eq!(r(&s, A).h, 540); // minimum lama dilupakan, diukur ulang
    }

    #[test]
    fn gap_around_single_window() {
        let mut s = strip();
        s.set_gap(20);
        s.add_window(A);
        assert_eq!(r(&s, A), rect(20, 20, 1880, 1040));
    }

    #[test]
    fn gap_between_lanes_equals_gap_setting() {
        let mut s = strip();
        s.set_gap(20);
        s.add_window(A);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.add_window(B);
        s.snap(Dir::Up);
        s.snap(Dir::Right);
        let (a, b) = (r(&s, A), r(&s, B));
        assert_eq!(b.x - (a.x + a.w), 20);
        assert_eq!(a.x, 20); // jarak ke tepi = gap
        assert_eq!(1920 - (b.x + b.w), 20);
    }

    #[test]
    fn gap_between_rows_equals_gap_setting() {
        let mut s = strip();
        s.set_gap(20);
        s.add_window(A);
        s.snap(Dir::Up);
        s.add_window(B);
        s.snap(Dir::Up);
        let (a, b) = (r(&s, A), r(&s, B));
        assert_eq!(b.y - (a.y + a.h), 20);
    }

    #[test]
    fn min_height_accounts_for_gap() {
        let mut s = strip();
        s.set_gap(20);
        s.add_window(A);
        s.snap(Dir::Up);
        s.snap(Dir::Up);
        s.set_min_height(A, 600);
        assert_eq!(r(&s, A).h, 600);
    }

    #[test]
    fn min_height_mismatch_is_evened_out_before_full_window() {
        // A kiri setengah, B kanan setengah, C penuh di bawah. B punya tinggi
        // minimum lebih besar -> A diregangkan supaya dasar A dan B sejajar.
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.add_window(B);
        s.snap(Dir::Up);
        s.snap(Dir::Right);
        s.add_window(C);
        s.set_min_height(B, 600);
        assert_eq!(r(&s, B).h, 600);
        assert_eq!(r(&s, A).h, 600);
        assert_eq!(r(&s, C).y - r(&s, B).y, 600);
    }

    #[test]
    fn intentionally_unaligned_lanes_are_not_stretched() {
        // A setengah di kiri, B sepertiga di kanan: memang tidak sejajar.
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.add_window(B);
        s.snap(Dir::Up);
        s.snap(Dir::Up);
        s.snap(Dir::Right);
        s.add_window(C);
        assert_eq!(r(&s, A).h, 540);
        assert_eq!(r(&s, B).h, 360);
    }

    #[test]
    fn empty_lane_is_not_filled() {
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.add_window(C);
        assert_eq!(r(&s, A).h, 540);
        assert_eq!(r(&s, C).y - r(&s, A).y, 540);
    }

    #[test]
    fn hidden_window_frees_its_space_and_returns_to_same_slot() {
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.add_window(B);
        s.snap(Dir::Up);
        s.snap(Dir::Right);
        assert_eq!(r(&s, B), rect(960, 0, 960, 540));
        assert!(s.set_hidden(B, true));
        assert_eq!(s.layout().len(), 1);
        assert_eq!(s.focused(), Some(A));
        assert!(s.set_hidden(B, false));
        assert_eq!(r(&s, B), rect(960, 0, 960, 540));
        assert!(!s.set_hidden(B, false)); // sudah terlihat
    }

    #[test]
    fn hiding_everything_then_showing_restores_the_whole_layout() {
        // 2x2 + satu window penuh di bawah, lalu "Win+D" dan buka lagi
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.add_window(B);
        s.snap(Dir::Up);
        s.snap(Dir::Right);
        s.add_window(C);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.add_window(D);
        s.snap(Dir::Up);
        s.snap(Dir::Right);
        s.add_window(X);
        let ids = [A, B, C, D, X];
        let shape = |s: &Strip| -> Vec<(i32, i32, i32, i32)> {
            let ay = r(s, A).y;
            ids.iter()
                .map(|&i| {
                    let q = r(s, i);
                    (q.x, q.y - ay, q.w, q.h)
                })
                .collect()
        };
        let before = shape(&s);
        for &i in &ids {
            s.set_hidden(i, true);
        }
        assert!(s.layout().is_empty());
        assert_eq!(s.focused(), None);
        for &i in ids.iter().rev() {
            s.set_hidden(i, false); // urutan muncul tidak harus sama
        }
        assert_eq!(shape(&s), before);
        assert!(s.focused().is_some());
    }

    #[test]
    fn hidden_window_is_skipped_by_focus_and_removal() {
        let mut s = strip();
        s.add_window(A);
        s.add_window(B);
        s.add_window(C);
        s.set_hidden(B, true);
        s.remove_window(C);
        assert_eq!(s.focused(), Some(A)); // B tersembunyi, tidak dipilih
        s.focus(Dir::Down);
        assert_eq!(s.focused(), Some(A)); // tidak ada tetangga terlihat
        s.set_focus(B); // diabaikan selama tersembunyi
        assert_eq!(s.focused(), Some(A));
    }

    #[test]
    fn overview_describes_whole_strip_including_offscreen_part() {
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.add_window(B);
        s.snap(Dir::Up);
        s.snap(Dir::Right);
        s.add_window(C);
        s.snap(Dir::Up);
        let ov = s.overview();
        let at = |id| ov.items.iter().find(|i| i.id == id).unwrap().rect;
        assert_eq!((ov.width, ov.height), (1920, 1080));
        assert_eq!(at(A), rect(0, 0, 960, 540));
        assert_eq!(at(B), rect(960, 0, 960, 540));
        assert_eq!(at(C), rect(0, 540, 1920, 540));

        s.add_window(D); // satu layar penuh lagi di bawah
        let ov = s.overview();
        assert_eq!(ov.height, 2160);
        assert_eq!(ov.view_y, 1080); // yang tampil: D saja
        assert_eq!(ov.view_h, 1080);
    }

    #[test]
    fn overview_skips_hidden_windows() {
        let mut s = strip();
        s.add_window(A);
        s.add_window(B);
        s.set_hidden(B, true);
        assert_eq!(s.overview().items.len(), 1);
    }

    #[test]
    fn neighbor_of_does_not_change_focus() {
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.add_window(B);
        s.snap(Dir::Up);
        s.snap(Dir::Right);
        assert_eq!(s.focused(), Some(B));
        assert_eq!(s.neighbor_of(A, Dir::Right), Some(B));
        assert_eq!(s.neighbor_of(B, Dir::Left), Some(A));
        assert_eq!(s.neighbor_of(A, Dir::Up), None);
        assert_eq!(s.focused(), Some(B));
    }

    #[test]
    fn cycle_follows_reading_order_and_wraps() {
        // A kiri atas, B kanan atas, C penuh di bawah
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Up);
        s.snap(Dir::Left);
        s.add_window(B);
        s.snap(Dir::Up);
        s.snap(Dir::Right);
        s.add_window(C);
        s.snap(Dir::Up);
        assert_eq!(s.cycle_from(A, true), Some(B));
        assert_eq!(s.cycle_from(B, true), Some(C));
        assert_eq!(s.cycle_from(C, true), Some(A)); // berputar
        assert_eq!(s.cycle_from(A, false), Some(C));
        assert_eq!(s.cycle_from(C, false), Some(B));
        assert_eq!(s.focused(), Some(C)); // fokus tidak berubah
    }

    #[test]
    fn cycle_skips_hidden_and_needs_two_windows() {
        let mut s = strip();
        s.add_window(A);
        assert_eq!(s.cycle_from(A, true), None);
        s.add_window(B);
        s.add_window(C);
        s.set_hidden(B, true);
        assert_eq!(s.cycle_from(A, true), Some(C));
        assert_eq!(s.cycle_from(B, true), None); // tersembunyi
    }
}
