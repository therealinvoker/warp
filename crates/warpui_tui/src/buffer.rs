use crate::TuiRect;
use crate::TuiSize;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cell {
    pub symbol: char,
}

impl Default for Cell {
    fn default() -> Self {
        Self { symbol: ' ' }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TuiBuffer {
    size: TuiSize,
    cells: Vec<Cell>,
}

impl TuiBuffer {
    pub fn new(size: TuiSize) -> Self {
        Self {
            size,
            cells: vec![Cell::default(); usize::from(size.width) * usize::from(size.height)],
        }
    }

    pub fn size(&self) -> TuiSize {
        self.size
    }

    pub fn set_symbol(&mut self, x: u16, y: u16, symbol: char) {
        if x >= self.size.width || y >= self.size.height {
            return;
        }

        let index = usize::from(y) * usize::from(self.size.width) + usize::from(x);
        self.cells[index].symbol = symbol;
    }

    pub fn write_str(&mut self, x: u16, y: u16, max_width: u16, text: &str) {
        for (offset, ch) in text.chars().take(usize::from(max_width)).enumerate() {
            let Ok(offset) = u16::try_from(offset) else {
                break;
            };
            self.set_symbol(x.saturating_add(offset), y, ch);
        }
    }

    pub fn fill_rect(&mut self, rect: TuiRect, symbol: char) {
        for y in rect.y..rect.bottom().min(self.size.height) {
            for x in rect.x..rect.right().min(self.size.width) {
                self.set_symbol(x, y, symbol);
            }
        }
    }

    pub fn lines(&self) -> Vec<String> {
        self.cells
            .chunks(usize::from(self.size.width))
            .map(|row| row.iter().map(|cell| cell.symbol).collect())
            .collect()
    }
}
