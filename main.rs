use chess::{
    Board, BoardStatus, ChessMove, Color, File, MoveGen,
    Piece, Rank, Square,
};

use eframe::egui;

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Command, Stdio},
    str::FromStr,
    sync::mpsc::{self, Receiver, TryRecvError},
    time::Duration,
};

struct Woodpush {
    board: Board,
    selected: Option<Square>,
    message: String,
    engine_path: PathBuf,
    textures: HashMap<String, egui::TextureHandle>,
    engine_receiver: Option<Receiver<Result<ChessMove, String>>>,
    engine_thinking: bool,
}

impl Woodpush {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut textures = HashMap::new();

        let pieces = [
            "white_king",
            "white_queen",
            "white_rock",
            "white_bishop",
            "white_knight",
            "white_pawn",
            "black_king",
            "black_queen",
            "black_rock",
            "black_bishop",
            "black_knight",
            "black_pawn",
        ];

        let mut load_errors = Vec::new();

        for name in pieces {
            let path = PathBuf::from("assets")
                .join("pieces")
                .join(format!("{}.webp", name));

            match image::open(&path) {
                Ok(img) => {
                    let rgba = img.to_rgba8();

                    let size = [
                        rgba.width() as usize,
                        rgba.height() as usize,
                    ];

                    let color_image =
                        egui::ColorImage::from_rgba_unmultiplied(
                            size,
                            rgba.as_raw(),
                        );

                    let texture = cc.egui_ctx.load_texture(
                        name,
                        color_image,
                        egui::TextureOptions::LINEAR,
                    );

                    textures.insert(name.to_string(), texture);
                }

                Err(error) => {
                    load_errors.push(format!(
                        "{}: {}",
                        path.display(),
                        error
                    ));
                }
            }
        }

        let message = if load_errors.is_empty() {
            "Все шахматные фигуры загружены.".to_string()
        } else {
            format!(
                "Не удалось загрузить некоторые фигуры:\n{}",
                load_errors.join("\n")
            )
        };

        Self {
            board: Board::default(),
            selected: None,
            message,
            engine_path: PathBuf::from(
                "C:\\Users\\User\\Desktop\\project\\stockfish-windows-x86-64-universal.exe"
            ),
            textures,
            engine_receiver: None,
            engine_thinking: false,
        }
    }

    fn piece_filename(piece: Piece, color: Color) -> &'static str {
        match (piece, color) {
            (Piece::King, Color::White) => "white_king",
            (Piece::Queen, Color::White) => "white_queen",
            (Piece::Rook, Color::White) => "white_rock",
            (Piece::Bishop, Color::White) => "white_bishop",
            (Piece::Knight, Color::White) => "white_knight",
            (Piece::Pawn, Color::White) => "white_pawn",

            (Piece::King, Color::Black) => "black_king",
            (Piece::Queen, Color::Black) => "black_queen",
            (Piece::Rook, Color::Black) => "black_rock",
            (Piece::Bishop, Color::Black) => "black_bishop",
            (Piece::Knight, Color::Black) => "black_knight",
            (Piece::Pawn, Color::Black) => "black_pawn",
        }
    }

    fn status_text(&self) -> String {
        match self.board.status() {
            BoardStatus::Checkmate => {
                if self.board.side_to_move() == Color::White {
                    "Мат! Победил Stockfish".to_string()
                } else {
                    "Мат! Победили белые".to_string()
                }
            }

            BoardStatus::Stalemate => {
                "Ничья — пат".to_string()
            }

            BoardStatus::Ongoing => {
                if self.board.side_to_move() == Color::White {
                    "Ваш ход".to_string()
                } else {
                    "Ход Stockfish".to_string()
                }
            }
        }
    }

    fn make_move(&mut self, chess_move: ChessMove) {
        self.board = self.board.make_move_new(chess_move);
        self.selected = None;
    }

    fn restart(&mut self) {
        self.board = Board::default();
        self.selected = None;

        // Отбрасываем ответ предыдущей партии.
        self.engine_receiver = None;
        self.engine_thinking = false;

        self.message = "Новая партия. Ваш ход.".to_string();
    }

    // Запуск Stockfish в отдельном потоке.
    fn engine_move(&mut self) {
        if self.board.status() != BoardStatus::Ongoing {
            return;
        }

        if self.board.side_to_move() != Color::Black {
            return;
        }

        if self.engine_thinking {
            return;
        }

        self.message = "Stockfish думает...".to_string();
        self.engine_thinking = true;

        let board = self.board;
        let engine_path = self.engine_path.clone();

        let (sender, receiver) = mpsc::channel();

        self.engine_receiver = Some(receiver);

        std::thread::spawn(move || {
            let result = Self::ask_stockfish(board, &engine_path);
            let _ = sender.send(result);
        });
    }

    // Получение результата от Stockfish.
    fn poll_engine(&mut self) {
        let result = match self.engine_receiver.as_ref() {
            Some(receiver) => match receiver.try_recv() {
                Ok(result) => Some(result),

                Err(TryRecvError::Empty) => None,

                Err(TryRecvError::Disconnected) => {
                    Some(Err(
                        "Поток Stockfish завершился.".to_string()
                    ))
                }
            },

            None => None,
        };

        if let Some(result) = result {
            self.engine_receiver = None;
            self.engine_thinking = false;

            match result {
                Ok(chess_move) => {
                    if MoveGen::new_legal(&self.board)
                        .any(|m| m == chess_move)
                    {
                        self.make_move(chess_move);
                        self.message = self.status_text();
                    } else {
                        self.message =
                            "Stockfish вернул нелегальный ход.".to_string();
                    }
                }

                Err(error) => {
                    self.message = format!(
                        "Ошибка Stockfish: {}",
                        error
                    );
                }
            }
        }
    }

    // Запрос хода у движка.
    fn ask_stockfish(
        board: Board,
        engine_path: &PathBuf,
    ) -> Result<ChessMove, String> {
        let mut child = Command::new(engine_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                format!(
                    "Не удалось запустить движок: {}. Проверь Stockfish.",
                    e
                )
            })?;

        let mut input = child.stdin.take()
            .ok_or("Не удалось открыть stdin Stockfish")?;

        let output = child.stdout.take()
            .ok_or("Не удалось открыть stdout Stockfish")?;

        let mut reader = BufReader::new(output);
        let mut line = String::new();

        writeln!(input, "uci").map_err(|e| e.to_string())?;
        input.flush().map_err(|e| e.to_string())?;

        loop {
            line.clear();

            if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                return Err("Stockfish завершил работу.".to_string());
            }

            if line.trim() == "uciok" {
                break;
            }
        }

        writeln!(input, "isready").map_err(|e| e.to_string())?;
        input.flush().map_err(|e| e.to_string())?;

        loop {
            line.clear();

            if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                return Err("Stockfish не ответил.".to_string());
            }

            if line.trim() == "readyok" {
                break;
            }
        }

        writeln!(input, "position fen {}", board)
            .map_err(|e| e.to_string())?;

        writeln!(input, "go movetime 1000")
            .map_err(|e| e.to_string())?;

        input.flush().map_err(|e| e.to_string())?;

        let best_move = loop {
            line.clear();

            if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                return Err("Нет ответа bestmove.".to_string());
            }

            let parts: Vec<&str> = line.split_whitespace().collect();

            if parts.first() == Some(&"bestmove") {
                break parts.get(1)
                    .ok_or("Stockfish не вернул ход")?
                    .to_string();
            }
        };

        drop(input);
        let _ = child.kill();
        let _ = child.wait();

        if best_move == "(none)" || best_move == "0000" {
            return Err("Нет доступных ходов.".to_string());
        }

        if best_move.len() < 4 {
            return Err("Некорректный формат хода.".to_string());
        }

        let from = Square::from_str(&best_move[0..2])
            .map_err(|_| "Некорректная начальная клетка")?;

        let to = Square::from_str(&best_move[2..4])
            .map_err(|_| "Некорректная конечная клетка")?;

        let promotion = if best_move.len() >= 5 {
            match &best_move[4..5] {
                "q" => Some(Piece::Queen),
                "r" => Some(Piece::Rook),
                "b" => Some(Piece::Bishop),
                "n" => Some(Piece::Knight),

                _ => {
                    return Err(
                        "Некорректное превращение пешки".to_string()
                    );
                }
            }
        } else {
            None
        };

        Ok(ChessMove::new(from, to, promotion))
    }

    fn handle_square(&mut self, square: Square) {
        if self.board.status() != BoardStatus::Ongoing {
            return;
        }

        if self.board.side_to_move() != Color::White {
            return;
        }

        if let Some(from) = self.selected {
            let found = MoveGen::new_legal(&self.board)
                .find(|m| {
                    m.get_source() == from
                        && m.get_dest() == square
                });

            if let Some(mut chess_move) = found {
                if self.board.piece_on(from) == Some(Piece::Pawn)
                    && square.get_rank() == Rank::Eighth
                {
                    chess_move = ChessMove::new(
                        from,
                        square,
                        Some(Piece::Queen),
                    );
                }

                self.make_move(chess_move);
                self.message = "Ход сделан.".to_string();

                self.engine_move();

                return;
            }
        }

        if self.board.color_on(square) == Some(Color::White) {
            self.selected = Some(square);
        } else {
            self.selected = None;
        }
    }

    // Определение клетки под курсором.
    fn square_from_pos(
        rect: egui::Rect,
        pos: egui::Pos2,
    ) -> Option<Square> {
        if !rect.contains(pos) {
            return None;
        }

        let cell_size = rect.width() / 8.0;

        let col = ((pos.x - rect.left()) / cell_size) as usize;
        let row = ((pos.y - rect.top()) / cell_size) as usize;

        if row >= 8 || col >= 8 {
            return None;
        }

        Some(Square::make_square(
            Rank::from_index(7 - row),
            File::from_index(col),
        ))
    }

    // Отрисовка шахматной доски с Drag & Drop.
    fn draw_board(&mut self, ui: &mut egui::Ui) {
        let legal_moves: Vec<ChessMove> =
            MoveGen::new_legal(&self.board).collect();

        let selected = self.selected;
        let board_size = 512.0;

        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(board_size, board_size),
            egui::Sense::click_and_drag(),
        );

        let cell_size = board_size / 8.0;

        let pointer_pos = ui.ctx().input(|i| i.pointer.interact_pos());

        let dragging = response.dragged();

        let painter = ui.painter();

        for row in 0..8 {
            for col in 0..8 {
                let square = Square::make_square(
                    Rank::from_index(7 - row),
                    File::from_index(col),
                );

                let light = (row + col) % 2 == 0;

                let mut color = if light {
                    egui::Color32::from_rgb(238, 238, 210)
                } else {
                    egui::Color32::from_rgb(118, 150, 86)
                };

                if selected == Some(square) {
                    color = egui::Color32::from_rgb(
                        246, 246, 105
                    );
                }

                let cell_rect = egui::Rect::from_min_size(
                    rect.min + egui::vec2(
                        col as f32 * cell_size,
                        row as f32 * cell_size,
                    ),
                    egui::vec2(cell_size, cell_size),
                );

                painter.rect_filled(cell_rect, 0.0, color);

                // Подсветка доступных ходов.
                if let Some(from) = selected {
                    if legal_moves.iter().any(|m| {
                        m.get_source() == from
                            && m.get_dest() == square
                    }) {
                        painter.circle_filled(
                            cell_rect.center(),
                            9.0,
                            egui::Color32::from_rgba_unmultiplied(
                                30, 30, 30, 110
                            ),
                        );
                    }
                }

                if let Some(piece) = self.board.piece_on(square) {
                    let piece_color =
                        self.board.color_on(square).unwrap();

                    let filename = Self::piece_filename(
                        piece,
                        piece_color,
                    );

                    // Во время перетаскивания прячем фигуру
                    // на исходной клетке.
                    let hide_source = dragging
                        && selected == Some(square);

                    if !hide_source {
                        if let Some(texture) =
                            self.textures.get(filename)
                        {
                            painter.image(
                                texture.id(),
                                cell_rect.shrink(4.0),
                                egui::Rect::from_min_max(
                                    egui::Pos2::ZERO,
                                    egui::Pos2::new(1.0, 1.0),
                                ),
                                egui::Color32::WHITE,
                            );
                        }
                    }
                }
            }
        }

        // Рисуем фигуру под курсором.
        if dragging {
            if let (Some(square), Some(pos)) = (selected, pointer_pos) {
                if let Some(piece) = self.board.piece_on(square) {
                    if let Some(piece_color) =
                        self.board.color_on(square)
                    {
                        let filename =
                            Self::piece_filename(piece, piece_color);

                        if let Some(texture) =
                            self.textures.get(filename)
                        {
                            let floating_rect =
                                egui::Rect::from_center_size(
                                    pos,
                                    egui::vec2(
                                        cell_size * 0.95,
                                        cell_size * 0.95,
                                    ),
                                );

                            painter.image(
                                texture.id(),
                                floating_rect,
                                egui::Rect::from_min_max(
                                    egui::Pos2::ZERO,
                                    egui::Pos2::new(1.0, 1.0),
                                ),
                                egui::Color32::WHITE,
                            );
                        }
                    }
                }
            }
        }

        // Начало перетаскивания.
        if response.drag_started() {
            if let Some(pos) = pointer_pos {
                if let Some(square) = Self::square_from_pos(rect, pos) {
                    if self.board.color_on(square) == Some(Color::White)
                        && self.board.side_to_move() == Color::White
                    {
                        self.selected = Some(square);
                    }
                }
            }
        }

        // Отпускание фигуры.
        if response.drag_stopped() {
            if let Some(pos) = pointer_pos {
                if let Some(square) = Self::square_from_pos(rect, pos) {
                    self.handle_square(square);
                } else {
                    self.selected = None;
                }
            }
        }

        // Обычные клики.
        if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                if let Some(square) = Self::square_from_pos(rect, pos) {
                    self.handle_square(square);
                }
            }
        }
    }
}

impl eframe::App for Woodpush {
    fn update(
        &mut self,
        ctx: &egui::Context,
        _frame: &mut eframe::Frame,
    ) {
        // Проверяем, ответил ли Stockfish.
        self.poll_engine();

        // Обновляем интерфейс во время расчёта движка.
        if self.engine_thinking {
            ctx.request_repaint_after(
                Duration::from_millis(16)
            );
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Woodpush");
            ui.label("Chess against Stockfish");

            ui.add_space(10.0);

            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(self.status_text());

                    ui.add_space(8.0);

                    self.draw_board(ui);

                    ui.add_space(12.0);

                    if ui.button("Новая партия").clicked() {
                        self.restart();
                    }
                });

                ui.add_space(20.0);

                ui.vertical(|ui| {
                    ui.heading("Партия");
                    ui.separator();

                    ui.label(format!(
                        "Ход: {}",
                        if self.board.side_to_move() == Color::White {
                            "Белые"
                        } else {
                            "Чёрные"
                        }
                    ));

                    ui.add_space(12.0);

                    ui.label("Соперник:");
                    ui.strong("Stockfish");

                    ui.add_space(12.0);

                    ui.label("Время на ход:");
                    ui.strong("1 секунда");

                    ui.add_space(20.0);

                    ui.label("Состояние:");
                    ui.label(self.status_text());

                    ui.add_space(12.0);

                    ui.label("Сообщение:");

                    if self.engine_thinking {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Stockfish анализирует позицию...");
                        });
                    } else {
                        ui.label(&self.message);
                    }

                    ui.add_space(20.0);

                    if ui.button("Перезапустить").clicked() {
                        self.restart();
                    }
                });
            });
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1050.0, 680.0])
            .with_min_inner_size([850.0, 600.0]),

        ..Default::default()
    };

    eframe::run_native(
        "Woodpush",
        options,
        Box::new(|cc| Ok(Box::new(Woodpush::new(cc)))),
    )
}
