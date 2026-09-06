//! Treemap - die Größenverhältnisse als Fläche
//!
//! Jeder Knoten bekommt ein Rechteck, dessen Fläche seiner Größe entspricht;
//! die Kinder eines Ordners teilen sein Rechteck unter sich auf. So sieht
//! man auf einen Blick, wo der Platz hingeht - ein 120-GB-VHDX ist ein
//! riesiger Block, hunderttausend kleine Dateien ein feines Muster.
//!
//! # Squarified Layout
//!
//! Eine Reihe von Kindern wird entlang der kürzeren Seite des Rechtecks
//! abgelegt, und die Reihe wächst so lange, wie ihre Rechtecke dadurch
//! quadratischer werden (Bruls, Huizing, van Wijk 2000). Quadratische
//! Kacheln sind leichter zu vergleichen und zu treffen als dünne Streifen.
//! Voraussetzung: die Kinder sind nach Größe absteigend sortiert - das sind
//! sie im Baum ohnehin.
//!
//! # Zeichnen als Bild
//!
//! Ein Laufwerk hat Millionen Knoten; als einzelne GUI-Elemente wäre das
//! nicht darstellbar. Stattdessen entsteht hier ein fertiges Pixelbild in
//! der Größe des Anzeigebereichs, das die GUI nur noch anzeigt. Knoten,
//! die kleiner als ein Pixel wären, werden samt Teilbaum übersprungen; ihr
//! Ordner bleibt an der Stelle sichtbar. Damit hängt der Aufwand an der
//! Bildgröße, nicht an der Zahl der Dateien.
//!
//! Zu jedem gezeichneten Rechteck merkt sich die Treemap die Kette der
//! MFT-Referenzen von der Wurzel bis zum Knoten. Damit findet die GUI zu
//! einem Mauspunkt den Knoten und seinen Pfad, ohne den Baum zu
//! durchsuchen.

use crate::tree::TreeNode;

/// Rechtecke unterhalb dieser Kantenlänge (in Pixeln) werden nicht mehr
/// unterteilt
const MIN_SIZE: f64 = 1.0;

/// Hintergrund von Ordnern und leeren Flächen
const DIRECTORY_COLOR: [u8; 3] = [0x2a, 0x2b, 0x3c];

/// Ein Rechteck in Pixelkoordinaten (nicht gerundet)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Ein gezeichnetes Rechteck mit dem Weg zu seinem Knoten
#[derive(Debug, Clone)]
struct HitRect {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    /// Tiefe unterhalb der Treemap-Wurzel (0 = die Wurzel selbst)
    depth: u16,
    /// Start und Länge der Kette in `chains`
    chain_start: u32,
    chain_len: u16,
}

/// Ein Treffer von [`Treemap::hit`]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit<'a> {
    /// MFT-Referenzen von der Treemap-Wurzel (ausgenommen) bis zum Knoten
    pub chain: &'a [u64],
}

/// Pixelgrenzen eines gezeichneten Rechtecks, Ende exklusiv
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

/// Eine fertig gezeichnete Treemap: Pixel plus Trefferliste
pub struct Treemap {
    width: u32,
    height: u32,
    /// RGB, zeilenweise, 3 Bytes pro Pixel
    pixels: Vec<u8>,
    rects: Vec<HitRect>,
    chains: Vec<u64>,
}

impl Treemap {
    /// Zeichnet den Teilbaum unter `root` in ein Bild von `width` x `height`
    /// Pixeln. `root` selbst ist die Fläche, seine Kinder die Kacheln.
    pub fn render(root: &TreeNode, width: u32, height: u32) -> Self {
        let mut map = Self {
            width,
            height,
            pixels: vec![0; width as usize * height as usize * 3],
            rects: Vec::new(),
            chains: Vec::new(),
        };
        if width == 0 || height == 0 {
            return map;
        }

        let full = Rect {
            x: 0.0,
            y: 0.0,
            w: width as f64,
            h: height as f64,
        };
        let mut chain = Vec::new();
        map.place(root, full, 0, &mut chain);
        map
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Die Pixel als RGB-Bytes, zeilenweise
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Anzahl gezeichneter Rechtecke (Ordner und Dateien)
    pub fn rect_count(&self) -> usize {
        self.rects.len()
    }

    /// Der tiefste Knoten unter dem Punkt, falls dort einer gezeichnet ist
    pub fn hit(&self, x: f64, y: f64) -> Option<Hit<'_>> {
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let (px, py) = (x as u32, y as u32);
        self.rects
            .iter()
            .filter(|rect| px >= rect.x0 && px < rect.x1 && py >= rect.y0 && py < rect.y1)
            .max_by_key(|rect| rect.depth)
            .map(|rect| Hit {
                chain: &self.chains
                    [rect.chain_start as usize..rect.chain_start as usize + rect.chain_len as usize],
            })
    }

    /// Das Rechteck des Knotens mit genau dieser Kette, falls gezeichnet
    /// (die Kette ist relativ zur Treemap-Wurzel, wie bei [`Treemap::hit`])
    pub fn bounds_of(&self, chain: &[u64]) -> Option<Bounds> {
        self.rects
            .iter()
            .find(|rect| {
                let start = rect.chain_start as usize;
                &self.chains[start..start + rect.chain_len as usize] == chain
            })
            .map(|rect| Bounds {
                x0: rect.x0,
                y0: rect.y0,
                x1: rect.x1,
                y1: rect.y1,
            })
    }

    /// Zeichnet einen Knoten in sein Rechteck und verteilt es auf die Kinder
    fn place(&mut self, node: &TreeNode, rect: Rect, depth: u16, chain: &mut Vec<u64>) {
        let Some((x0, y0, x1, y1)) = pixel_bounds(rect) else {
            return; // kleiner als ein Pixel
        };

        if node.is_directory {
            self.fill(x0, y0, x1, y1, DIRECTORY_COLOR);
        } else {
            self.fill_cushion(x0, y0, x1, y1, file_color(&node.name));
        }
        self.remember(x0, y0, x1, y1, depth, chain);

        if !node.is_directory || rect.w < MIN_SIZE || rect.h < MIN_SIZE {
            return;
        }

        // Zu kleine Kinder sind für das Layout trotzdem da (ihre Fläche
        // gehört zum Ordner), gezeichnet werden sie nicht
        let mut placed = Vec::new();
        layout(node.children.iter().map(|child| child.total_size), rect, &mut placed);
        for (index, child_rect) in placed {
            let child = &node.children[index];
            chain.push(child.id);
            self.place(child, child_rect, depth + 1, chain);
            chain.pop();
        }
    }

    fn remember(&mut self, x0: u32, y0: u32, x1: u32, y1: u32, depth: u16, chain: &[u64]) {
        let chain_start = self.chains.len() as u32;
        self.chains.extend_from_slice(chain);
        self.rects.push(HitRect {
            x0,
            y0,
            x1,
            y1,
            depth,
            chain_start,
            chain_len: chain.len() as u16,
        });
    }

    fn fill(&mut self, x0: u32, y0: u32, x1: u32, y1: u32, color: [u8; 3]) {
        for y in y0..y1 {
            let row = y as usize * self.width as usize;
            for x in x0..x1 {
                let offset = (row + x as usize) * 3;
                self.pixels[offset..offset + 3].copy_from_slice(&color);
            }
        }
    }

    /// Füllt mit einem Kissen-Verlauf: hell in der Mitte, dunkel zum Rand -
    /// so bleiben auch gleichfarbige Nachbarn unterscheidbar
    fn fill_cushion(&mut self, x0: u32, y0: u32, x1: u32, y1: u32, color: [u8; 3]) {
        let (w, h) = ((x1 - x0) as f64, (y1 - y0) as f64);
        for y in y0..y1 {
            let fy = (y - y0) as f64 + 0.5;
            let ty = 1.0 - (2.0 * fy / h - 1.0).powi(2);
            let row = y as usize * self.width as usize;
            for x in x0..x1 {
                let fx = (x - x0) as f64 + 0.5;
                let tx = 1.0 - (2.0 * fx / w - 1.0).powi(2);
                let light = 0.45 + 0.65 * (tx * ty).sqrt();
                let offset = (row + x as usize) * 3;
                for channel in 0..3 {
                    self.pixels[offset + channel] =
                        (color[channel] as f64 * light).round().min(255.0) as u8;
                }
            }
        }
    }
}

/// Rundet ein Rechteck auf ganze Pixel; `None`, wenn nichts übrig bleibt
fn pixel_bounds(rect: Rect) -> Option<(u32, u32, u32, u32)> {
    let x0 = rect.x.round().max(0.0) as u32;
    let y0 = rect.y.round().max(0.0) as u32;
    let x1 = (rect.x + rect.w).round().max(0.0) as u32;
    let y1 = (rect.y + rect.h).round().max(0.0) as u32;
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
}

/// Squarified Layout: verteilt `rect` auf die Größen, absteigend sortiert
///
/// Liefert (Index, Rechteck) für jede Größe > 0 in Reihenfolge der Eingabe.
/// Öffentlich, weil unabhängig vom Baum testbar.
pub fn layout(sizes: impl IntoIterator<Item = u64>, rect: Rect, out: &mut Vec<(usize, Rect)>) {
    let sizes: Vec<f64> = sizes.into_iter().map(|size| size as f64).collect();
    let total: f64 = sizes.iter().sum();
    if total <= 0.0 || rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let scale = rect.w * rect.h / total;

    let mut remaining = rect;
    let mut start = 0;
    while start < sizes.len() {
        if sizes[start] <= 0.0 {
            break; // absteigend sortiert: der Rest ist leer
        }
        let side = remaining.w.min(remaining.h);
        if side <= 0.0 {
            break;
        }

        // Reihe verlängern, solange das schlechteste Seitenverhältnis
        // darin nicht schlechter wird
        let mut end = start;
        let mut row_area = 0.0;
        let mut min_area = f64::INFINITY;
        let mut max_area = 0.0f64;
        let mut worst_so_far = f64::INFINITY;
        while end < sizes.len() && sizes[end] > 0.0 {
            let area = sizes[end] * scale;
            let new_row = row_area + area;
            let new_min = min_area.min(area);
            let new_max = max_area.max(area);
            let worst = (side * side * new_max / (new_row * new_row))
                .max(new_row * new_row / (side * side * new_min));
            if end > start && worst > worst_so_far {
                break;
            }
            row_area = new_row;
            min_area = new_min;
            max_area = new_max;
            worst_so_far = worst;
            end += 1;
        }

        // Reihe entlang der kürzeren Seite ablegen
        if remaining.w >= remaining.h {
            let strip = row_area / remaining.h;
            let mut y = remaining.y;
            for (i, size) in sizes.iter().enumerate().take(end).skip(start) {
                let h = size * scale / strip;
                out.push((i, Rect { x: remaining.x, y, w: strip, h }));
                y += h;
            }
            remaining.x += strip;
            remaining.w -= strip;
        } else {
            let strip = row_area / remaining.w;
            let mut x = remaining.x;
            for (i, size) in sizes.iter().enumerate().take(end).skip(start) {
                let w = size * scale / strip;
                out.push((i, Rect { x, y: remaining.y, w, h: strip }));
                x += w;
            }
            remaining.y += strip;
            remaining.h -= strip;
        }
        start = end;
    }
}

/// Farbe einer Datei nach ihrer Endung: gleiche Endung, gleiche Farbe -
/// so springen z.B. alle Videos oder alle Images ins Auge
fn file_color(name: &str) -> [u8; 3] {
    let extension = name
        .rsplit_once('.')
        .map(|(stem, ext)| if stem.is_empty() { "" } else { ext })
        .unwrap_or("");
    if extension.is_empty() {
        return [0x9a, 0x9e, 0xb8];
    }

    // FNV-1a über die kleingeschriebene Endung, daraus ein Farbton
    let mut hash: u32 = 0x811c_9dc5;
    for byte in extension.bytes().map(|b| b.to_ascii_lowercase()) {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hsl_to_rgb((hash % 360) as f64, 0.55, 0.58)
}

/// HSL nach RGB, h in Grad, s und l in 0..1
fn hsl_to_rgb(h: f64, s: f64, l: f64) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn layout_covers_the_area_exactly() {
        let mut placed = Vec::new();
        layout([6, 6, 4, 3, 2, 2, 1], rect(0.0, 0.0, 600.0, 400.0), &mut placed);
        assert_eq!(placed.len(), 7);
        let area: f64 = placed.iter().map(|(_, r)| r.w * r.h).sum();
        assert!((area - 240_000.0).abs() < 1e-6);
        // Jedes Rechteck liegt innerhalb der Fläche
        for (_, r) in &placed {
            assert!(r.x >= -1e-9 && r.y >= -1e-9);
            assert!(r.x + r.w <= 600.0 + 1e-9 && r.y + r.h <= 400.0 + 1e-9);
        }
    }

    #[test]
    fn layout_matches_the_paper_example() {
        // Bruls et al.: 6,6,4,3,2,2,1 auf 6x4 - erste Reihe sind die beiden 6er
        let mut placed = Vec::new();
        layout([6, 6, 4, 3, 2, 2, 1], rect(0.0, 0.0, 6.0, 4.0), &mut placed);
        let (i, first) = placed[0];
        assert_eq!(i, 0);
        assert!((first.w - 3.0).abs() < 1e-9 && (first.h - 2.0).abs() < 1e-9);
        let (_, second) = placed[1];
        assert!((second.w - 3.0).abs() < 1e-9 && (second.h - 2.0).abs() < 1e-9);
        assert!((second.y - 2.0).abs() < 1e-9);
    }

    #[test]
    fn layout_skips_empty_sizes() {
        let mut placed = Vec::new();
        layout([10, 0, 0], rect(0.0, 0.0, 10.0, 10.0), &mut placed);
        assert_eq!(placed.len(), 1);
        layout([0, 0], rect(0.0, 0.0, 10.0, 10.0), &mut placed);
        assert_eq!(placed.len(), 1);
    }

    #[test]
    fn render_and_hit_find_the_biggest_file() {
        let mut root = TreeNode::new_directory("C:".to_string());
        root.id = 5;
        let mut big = TreeNode::new_directory("big".to_string());
        big.id = 10;
        let mut file = TreeNode::new_file("huge.vhdx".to_string(), 900);
        file.id = 11;
        big.add_child(file);
        let mut small = TreeNode::new_file("readme.txt".to_string(), 100);
        small.id = 20;
        root.add_child(big);
        root.add_child(small);
        root.sort_by_size();

        let map = Treemap::render(&root, 100, 50);
        assert_eq!(map.pixels().len(), 100 * 50 * 3);
        assert_eq!(map.rect_count(), 4);

        // Der große Ordner nimmt 90 % der Breite, links
        let hit = map.hit(10.0, 25.0).unwrap();
        assert_eq!(hit.chain, &[10, 11]);
        let hit = map.hit(95.0, 25.0).unwrap();
        assert_eq!(hit.chain, &[20]);
        assert!(map.hit(-1.0, 0.0).is_none());
        assert!(map.hit(1000.0, 0.0).is_none());

        // Rechteck zur Kette: die kleine Datei sitzt rechts, volle Höhe
        let bounds = map.bounds_of(&[20]).unwrap();
        assert_eq!((bounds.x0, bounds.y0, bounds.x1, bounds.y1), (90, 0, 100, 50));
        assert!(map.bounds_of(&[99]).is_none());
        assert_eq!(map.bounds_of(&[]).map(|b| b.x1), Some(100));

        // Gezeichnet, nicht schwarz
        let pixel = &map.pixels()[(25 * 100 + 10) * 3..(25 * 100 + 10) * 3 + 3];
        assert!(pixel.iter().any(|&c| c > 0));
    }

    #[test]
    fn tiny_nodes_are_skipped_but_parent_stays() {
        let mut root = TreeNode::new_directory("C:".to_string());
        let mut folder = TreeNode::new_directory("many".to_string());
        folder.id = 7;
        for i in 0..1000 {
            let mut f = TreeNode::new_file(format!("{}.dat", i), 1);
            f.id = 100 + i;
            folder.add_child(f);
        }
        root.add_child(folder);

        let map = Treemap::render(&root, 8, 8);
        // 64 Pixel für 1000 Dateien: die meisten fallen weg, der Ordner bleibt
        assert!(map.rect_count() < 100);
        assert_eq!(map.hit(4.0, 4.0).unwrap().chain[0], 7);
    }

    #[test]
    fn colors_depend_on_extension_only() {
        assert_eq!(file_color("a.MP4"), file_color("b.mp4"));
        assert_ne!(file_color("a.mp4"), file_color("a.txt"));
        assert_eq!(file_color("Makefile"), file_color(".gitignore"));
        assert_eq!(hsl_to_rgb(0.0, 0.0, 1.0), [255, 255, 255]);
    }
}
