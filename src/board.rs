use rand::seq::SliceRandom;
use serde::Deserialize;

#[derive(Clone, Copy, Default, Deserialize)]
pub struct Cell {
    pub mine: bool,
    pub revealed: bool,
    pub flagged: bool,
    pub count: u8,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Status {
    Playing,
    Won,
    Lost,
}

pub struct Board {
    pub rows: usize,
    pub columns: usize,
    pub mines: usize,
    pub cells: Vec<Cell>,
    pub status: Status,
    pub started: bool,
}

impl Board {
    pub fn new(rows: usize, columns: usize, mines: usize) -> Self {
        assert!(rows > 0 && columns > 0 && mines > 0 && mines < rows * columns);
        Self {
            rows,
            columns,
            mines,
            cells: vec![Cell::default(); rows * columns],
            status: Status::Playing,
            started: false,
        }
    }

    pub(crate) fn neighbors(&self, index: usize) -> Vec<usize> {
        let row = index / self.columns;
        let column = index % self.columns;
        let mut result = Vec::new();
        for r in row.saturating_sub(1)..=(row + 1).min(self.rows - 1) {
            for c in column.saturating_sub(1)..=(column + 1).min(self.columns - 1) {
                let neighbor = r * self.columns + c;
                if neighbor != index {
                    result.push(neighbor);
                }
            }
        }
        result
    }

    fn plant(&mut self, first: usize) {
        let mut candidates: Vec<_> = (0..self.cells.len()).filter(|&i| i != first).collect();
        candidates.shuffle(&mut rand::rng());
        for &index in candidates.iter().take(self.mines) {
            self.cells[index].mine = true;
        }
        for index in 0..self.cells.len() {
            self.cells[index].count = self
                .neighbors(index)
                .iter()
                .filter(|&&neighbor| self.cells[neighbor].mine)
                .count() as u8;
        }
        self.started = true;
    }

    pub fn flag(&mut self, index: usize) {
        if self.status == Status::Playing && !self.cells[index].revealed {
            self.cells[index].flagged = !self.cells[index].flagged;
        }
    }

    pub fn reveal(&mut self, index: usize) {
        if self.status != Status::Playing || self.cells[index].flagged {
            return;
        }
        if !self.started {
            self.plant(index);
        }
        if self.cells[index].revealed {
            let neighbors = self.neighbors(index);
            let flags = neighbors.iter().filter(|&&i| self.cells[i].flagged).count();
            if flags == usize::from(self.cells[index].count) {
                for neighbor in neighbors {
                    if !self.cells[neighbor].revealed && !self.cells[neighbor].flagged {
                        self.open(neighbor);
                        if self.status == Status::Lost {
                            return;
                        }
                    }
                }
            }
        } else {
            self.open(index);
        }
        if self.status != Status::Lost && self.cells.iter().all(|cell| cell.mine || cell.revealed) {
            self.status = Status::Won;
        }
    }

    fn open(&mut self, index: usize) {
        let mut pending = vec![index];
        while let Some(index) = pending.pop() {
            if self.cells[index].revealed || self.cells[index].flagged {
                continue;
            }
            self.cells[index].revealed = true;
            if self.cells[index].mine {
                self.status = Status::Lost;
                return;
            }
            if self.cells[index].count == 0 {
                pending.extend(self.neighbors(index));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_reveal_is_safe_and_mine_count_is_exact() {
        for first in 0..81 {
            let mut board = Board::new(9, 9, 10);
            board.reveal(first);
            assert!(!board.cells[first].mine);
            assert_eq!(board.cells.iter().filter(|cell| cell.mine).count(), 10);
            assert_ne!(board.status, Status::Lost);
        }
    }

    #[test]
    fn flags_prevent_reveals_and_do_not_start_clock() {
        let mut board = Board::new(9, 9, 10);
        board.flag(0);
        board.reveal(0);
        assert!(!board.started);
        assert!(!board.cells[0].revealed);
        board.flag(0);
        board.reveal(0);
        assert!(board.started);
    }

    fn fixed_board() -> Board {
        let mut board = Board::new(2, 2, 1);
        board.started = true;
        board.cells[0].mine = true;
        for index in 1..4 {
            board.cells[index].count = 1;
        }
        board
    }

    #[test]
    fn chording_wins_when_flags_are_correct() {
        let mut board = fixed_board();
        board.reveal(3);
        board.flag(0);
        board.reveal(3);
        assert_eq!(board.status, Status::Won);
    }

    #[test]
    fn incorrect_chord_loses_and_finished_games_ignore_actions() {
        let mut board = fixed_board();
        board.reveal(3);
        board.flag(1);
        board.reveal(3);
        assert_eq!(board.status, Status::Lost);
        board.flag(2);
        assert!(!board.cells[2].flagged);
    }

    #[test]
    fn flood_fill_reveals_connected_empty_cells() {
        let mut board = Board::new(3, 3, 1);
        board.started = true;
        board.cells[0].mine = true;
        for index in [1, 3, 4] {
            board.cells[index].count = 1;
        }
        board.reveal(8);
        assert_eq!(board.status, Status::Won);
        assert!(!board.cells[0].revealed);
    }
}
