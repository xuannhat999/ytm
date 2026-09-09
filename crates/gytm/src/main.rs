use api::{
    YTBus, YTDao,
    protocol::{ApiCmd, ApiResponse},
};
use config::Config;
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use data::mpv::{MpvCommand, MpvEvent};
use error::{YResult, log_to_file, startup_error_message};
use player::Player;
use ratatui::{Terminal, backend::CrosstermBackend};
use state::PlayerState;
use std::{env, io, time::Duration};
use tokio::sync::mpsc::{self};
use tui::{
    app::App,
    handler,
    helper::remove_queue_file,
    ui::{self},
    worker::spawn_api_worker,
};

#[tokio::main]
async fn main() -> YResult<()> {
    let mut player = Player::default();
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 && args[1] == "quit" {
        player.shutdown();
        remove_queue_file();
        println!("Exited gytm");
        std::process::exit(0);
    }
    // Setup App State
    let mut state = match PlayerState::load() {
        Ok(c) => c,
        Err(e) => {
            println!("{}", e);
            std::process::exit(1);
        }
    };

    let config = Config::load();
    // Setup API client
    let (api_cmd_tx, api_cmd_rx) = mpsc::unbounded_channel::<ApiCmd>();
    let (api_res_tx, mut api_res_rx) = mpsc::unbounded_channel::<ApiResponse>();
    let mut app = App::new(&state, &config, api_cmd_tx);

    println!("󱘖 Connecting to YouTube Music...");
    let dao = match YTDao::new().await {
        Ok(d) => d,
        Err(e) => {
            log_to_file(&e);
            eprintln!("{}", startup_error_message(&e));
            std::process::exit(1);
        }
    };
    let is_authed = dao.sapisid.is_some();
    let bus = YTBus::new(dao);
    spawn_api_worker(api_cmd_rx, api_res_tx, bus);
    if is_authed {
        app.api_cmd_tx.send(ApiCmd::FetchLibraryData).ok();
        app.api_loading_kind = Some(api::protocol::ApiLoadingKind::FetchLibraryData);
    }
    // Setup MPV player
    let (tx_event, mut rx) = mpsc::channel::<MpvEvent>(32);

    if Player::check_socket_exists()
        && let Ok(stream) = player.connect_mpv().await
    {
        app.load_queue_file().ok();
        player.observe_mpv(stream, tx_event).await?;
    } else {
        remove_queue_file();
        player.spawn_mpv()?;
        let stream = player.connect_mpv().await?;
        player.observe_mpv(stream, tx_event).await?;
        player.send_mpv_command(MpvCommand::SetVol(app.volume))?;
    }

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    if !is_authed {
        app.noti.notify(
            tui::notification::NotifyType::Error,
            "Running in logged-out mode. Library features are unavailable".to_string(),
        );
    }

    let mut render = true;
    let mut last_tick = std::time::Instant::now();
    let start_time = std::time::Instant::now();
    loop {
        let had_notification = app.noti.has_notification();
        let elapsed = std::cmp::min(last_tick.elapsed(), Duration::from_millis(100));
        last_tick = std::time::Instant::now();
        app.noti.tick(elapsed);
        while let Ok(event) = rx.try_recv() {
            handler::handle_mpv_event(&mut app, &mut state, event);
            render = true;
        }
        while let Ok(response) = api_res_rx.try_recv() {
            handler::handle_api_response(&mut app, response, &player);
            render = true;
        }
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    handler::handle_key_events(key, &mut app, &mut player, &mut state, &config);
                    render = true;
                }
                Event::Resize(_, _) => {
                    render = true;
                }
                _ => {}
            }
        }
        if app.is_exit {
            break;
        }

        if app.noti.has_notification() || had_notification {
            render = true;
        }
        if app.api_loading_kind.is_some() {
            render = true;
        }
        if render {
            terminal.draw(|f| {
                ui::render(&mut app, f, &config, start_time);
                app.noti.render(f, f.area());
            })?;
            render = false;
        }
    }
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}
