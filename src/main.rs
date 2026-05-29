mod gpio;
mod mcp;
mod neo;

use mcp::MCPController;
use neo::NeoPixelStrip;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use axum::{
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, State},
    response::Html,
    routing::get,
    Router,
};
use tokio::sync::broadcast;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;

// Beautiful 8-color palette (vibrant, rich hues)
const COLOR_PALETTE: [(u8, u8, u8); 8] = [
    (0, 0, 150),     // 0: Blue (Startup default)
    (150, 0, 0),     // 1: Red
    (0, 150, 0),     // 2: Green
    (150, 120, 0),   // 3: Yellow
    (120, 0, 150),   // 4: Purple/Magenta
    (0, 150, 150),   // 5: Cyan
    (150, 60, 0),    // 6: Orange
    (120, 120, 120), // 7: White
];

struct SystemState {
    colors: [(u8, u8, u8); 64],
    cursor_index: usize,
    active_color_index: usize,
    relay_state: u8,
}

struct HardwareControllers {
    strip: Option<NeoPixelStrip>,
    mcp: Option<MCPController>,
}

struct AppContext {
    state: Mutex<SystemState>,
    hardware: Mutex<Option<HardwareControllers>>,
    tx: broadcast::Sender<String>,
}

impl AppContext {
    /// Writes the current software state directly to the physical SPI NeoPixels and I2C Relays
    fn update_hardware(&self) {
        let state = self.state.lock().unwrap();
        let mut hw_lock = self.hardware.lock().unwrap();
        if let Some(ref mut hw) = *hw_lock {
            // Update NeoPixel strip
            if let Some(ref mut s) = hw.strip {
                let _ = s.show(&state.colors);
            }
            // Update Relays
            if let Some(ref mut mcp) = hw.mcp {
                let _ = mcp.set_relays(state.relay_state);
            }
        }
    }

    /// Serializes the state and broadcasts it to all connected WebSocket browsers
    fn broadcast_state(&self) {
        let state = self.state.lock().unwrap();
        let payload = serde_json::json!({
            "type": "state",
            "colors": &state.colors[..],
            "cursor": state.cursor_index,
            "active_color": state.active_color_index,
            "relay_state": state.relay_state
        });
        if let Ok(msg) = serde_json::to_string(&payload) {
            let _ = self.tx.send(msg);
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum ClientCommand {
    #[serde(rename = "paint")]
    Paint { index: usize, color_index: usize },
    #[serde(rename = "select_color")]
    SelectColor { color_index: usize },
    #[serde(rename = "toggle_relay")]
    ToggleRelay { relay_index: usize },
    #[serde(rename = "fill_all")]
    FillAll { color_index: usize },
    #[serde(rename = "clear_all")]
    ClearAll,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("--- 🦀 NeuraSync Low-Level Web Control Suite ---");

    // 1. Release hardware resets via GPIO RP1 chip
    if let Err(e) = gpio::initialize_mcp_resets() {
        eprintln!(
            "[Warning] GPIO Hardware resets failed: {}. Continuing in case pins are held high...",
            e
        );
    }

    // 2. Initialize physical NeoPixel strip (defaults to Blue startup colors)
    let strip = match NeoPixelStrip::new(64) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!(
                "[Warning] NeoPixel SPI initialization failed: {}.\n\
                 Ensure SPI is enabled. Running in virtual LED simulation mode.",
                e
            );
            None
        }
    };

    // 3. Initialize I2C MCP23017 controller (Enables built-in pull-ups for Button board)
    let mcp = match MCPController::new(1, 0x20) {
        Ok(mut m) => {
            if let Err(e) = m.initialize() {
                eprintln!("[Warning] MCP23017 Initialization failed: {}.", e);
            }
            Some(m)
        }
        Err(e) => {
            eprintln!(
                "[Warning] MCP23017 not detected on /dev/i2c-1: {}.\n\
                 Running in virtual MCP simulation mode.",
                e
            );
            None
        }
    };

    // 4. Initialize global App Context (Startup color: ALL BLUE)
    let context = Arc::new(AppContext {
        state: Mutex::new(SystemState {
            colors: [(0, 0, 150); 64], // Solid Blue on startup
            cursor_index: 0,
            active_color_index: 0,
            relay_state: 0x00,
        }),
        hardware: Mutex::new(Some(HardwareControllers { strip, mcp })),
        tx: broadcast::channel(100).0,
    });

    // Write initial Blue state to hardware immediately
    context.update_hardware();

    // 5. Spawn background task to poll MCP23017 physical buttons with soft debounce
    let context_hw = Arc::clone(&context);
    tokio::task::spawn_blocking(move || {
        let mut prev_buttons = 0xFF; // Start with all buttons released (internal pull-ups high)
        println!("[Hardware] Physical Button Polling Thread Started.");
        
        loop {
            let buttons_opt = {
                let mut hw_lock = context_hw.hardware.lock().unwrap();
                if let Some(ref mut hw) = *hw_lock {
                    if let Some(ref mut mcp) = hw.mcp {
                        mcp.read_buttons().ok()
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            if let Some(buttons) = buttons_opt {
                if buttons != prev_buttons {
                    // Soft Debounce Delay
                    std::thread::sleep(Duration::from_millis(25));
                    
                    let confirmed_buttons_opt = {
                        let mut hw_lock = context_hw.hardware.lock().unwrap();
                        if let Some(ref mut hw) = *hw_lock {
                            if let Some(ref mut mcp) = hw.mcp {
                                mcp.read_buttons().ok()
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    };

                    if let Some(confirmed_buttons) = confirmed_buttons_opt {
                        if confirmed_buttons == buttons {
                            // Detect transition from 1 (released) to 0 (pressed)
                            let pressed_mask = prev_buttons & !confirmed_buttons;

                            for i in 0..8 {
                                if (pressed_mask & (1 << i)) != 0 {
                                    println!("[Hardware] Physical Button {} (Color: {:?}) Pressed!", i, i);
                                    
                                    let mut state = context_hw.state.lock().unwrap();
                                    
                                    // Paint current cursor with the color mapping to the button
                                    let cursor = state.cursor_index;
                                    state.colors[cursor] = COLOR_PALETTE[i];
                                    
                                    // Toggle corresponding relay to trigger satisfying clicks
                                    state.relay_state ^= 1 << i;

                                    // Advance the cursor
                                    state.cursor_index = (state.cursor_index + 1) % 64;

                                    drop(state);

                                    // Apply to physical strip & relays
                                    context_hw.update_hardware();
                                    
                                    // Synchronize active screens in real-time
                                    context_hw.broadcast_state();
                                }
                            }
                            prev_buttons = confirmed_buttons;
                        }
                    }
                }
            }

            // Sleep to keep CPU consumption at 0%
            std::thread::sleep(Duration::from_millis(35));
        }
    });

    // 6. Graceful Ctrl+C Interrupt Handler
    let context_shutdown = Arc::clone(&context);
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap();
        println!("\n[System] Shutdown signal received. Blanking hardware peripherals cleanly...");
        
        // Blank states in memory
        {
            let mut state = context_shutdown.state.lock().unwrap();
            state.colors = [(0, 0, 0); 64];
            state.relay_state = 0x00;
        }

        // Apply blank states to hardware
        context_shutdown.update_hardware();
        println!("[System] Hardware powered down. Goodbye!");
        std::process::exit(0);
    });

    // 7. Define lightweight Axum HTTP Routing
    let app = Router::new()
        .route("/", get(serve_index))
        .route("/ws", get(ws_handler))
        .with_state(context);

    let port = 8080;
    println!("\n=======================================================");
    println!("  💡 NeuraSync Interface Active!");
    println!("  🌐 Point your browser to: http://<pi-ip-address>:{}", port);
    println!("  💻 Testing locally: http://localhost:{}", port);
    println!("=======================================================\n");

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn serve_index() -> Html<&'static str> {
    Html(HTML_CONTENT)
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(context): State<Arc<AppContext>>,
) -> impl axum::response::IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, context))
}

async fn handle_socket(socket: WebSocket, context: Arc<AppContext>) {
    let (mut sender, mut receiver) = socket.split();

    // Send initial system state to the newly connected browser
    let initial_msg = {
        let state = context.state.lock().unwrap();
        serde_json::json!({
            "type": "state",
            "colors": &state.colors[..],
            "cursor": state.cursor_index,
            "active_color": state.active_color_index,
            "relay_state": state.relay_state
        }).to_string()
    };
    let _ = sender.send(Message::Text(initial_msg)).await;

    // Subscribe to global broadcast channel
    let mut rx = context.tx.subscribe();
    
    // Spawn task to forward system updates to the browser
    let sender_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            if sender.send(Message::Text(msg)).await.is_err() {
                break;
            }
        }
    });

    // Listen to messages from this browser in a loop
    while let Some(Ok(Message::Text(text))) = receiver.next().await {
        if let Ok(cmd) = serde_json::from_str::<ClientCommand>(&text) {
            let mut state_changed = false;

            {
                let mut state = context.state.lock().unwrap();
                match cmd {
                    ClientCommand::Paint { index, color_index } => {
                        if index < 64 && color_index < 8 {
                            state.colors[index] = COLOR_PALETTE[color_index];
                            state.cursor_index = index; // Move cursor to active paint location
                            state_changed = true;
                        }
                    }
                    ClientCommand::SelectColor { color_index } => {
                        if color_index < 8 {
                            state.active_color_index = color_index;
                            state_changed = true;
                        }
                    }
                    ClientCommand::ToggleRelay { relay_index } => {
                        if relay_index < 8 {
                            state.relay_state ^= 1 << relay_index;
                            state_changed = true;
                        }
                    }
                    ClientCommand::FillAll { color_index } => {
                        if color_index < 8 {
                            state.colors = [COLOR_PALETTE[color_index]; 64];
                            state_changed = true;
                        }
                    }
                    ClientCommand::ClearAll => {
                        state.colors = [(0, 0, 0); 64];
                        state_changed = true;
                    }
                }
            }

            if state_changed {
                context.update_hardware();
                context.broadcast_state();
            }
        }
    }

    sender_task.abort();
}

// Visual masterpiece HTML template (embedded directly)
const HTML_CONTENT: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>NeuraSync NeoPixel Matrix Control</title>
    <link href="https://fonts.googleapis.com/css2?family=Outfit:wght@300;400;600;800&display=swap" rel="stylesheet">
    <style>
        :root {
            --bg-primary: #080a0f;
            --bg-glass: rgba(13, 17, 24, 0.7);
            --border-glass: rgba(255, 255, 255, 0.08);
            --neon-blue: #00d2ff;
            --neon-green: #00ff87;
            --neon-pink: #ff007f;
            --text-main: #f0f4f8;
            --text-muted: #8a99ad;
        }

        * {
            box-sizing: border-box;
            margin: 0;
            padding: 0;
        }

        body {
            font-family: 'Outfit', sans-serif;
            background: radial-gradient(circle at 50% 50%, #111827 0%, #030712 100%);
            color: var(--text-main);
            min-height: 100vh;
            display: flex;
            flex-direction: column;
            align-items: center;
            padding: 40px 20px;
            overflow-y: auto;
        }

        .title-container {
            text-align: center;
            margin-bottom: 30px;
        }

        h1 {
            font-size: 2.8rem;
            font-weight: 800;
            background: linear-gradient(135deg, #00d2ff 0%, #00ff87 100%);
            -webkit-background-clip: text;
            -webkit-text-fill-color: transparent;
            letter-spacing: -1px;
            margin-bottom: 8px;
            text-shadow: 0 0 30px rgba(0, 210, 255, 0.2);
        }

        .subtitle {
            color: var(--text-muted);
            font-size: 1.1rem;
            font-weight: 300;
        }

        .dashboard {
            display: grid;
            grid-template-columns: 1fr;
            gap: 30px;
            max-width: 1100px;
            width: 100%;
        }

        @media (min-width: 900px) {
            .dashboard {
                grid-template-columns: 1.2fr 1fr;
            }
        }

        .panel {
            background: var(--bg-glass);
            backdrop-filter: blur(16px);
            -webkit-backdrop-filter: blur(16px);
            border: 1px solid var(--border-glass);
            border-radius: 28px;
            padding: 30px;
            box-shadow: 0 20px 50px rgba(0, 0, 0, 0.4);
            display: flex;
            flex-direction: column;
            align-items: center;
        }

        .panel-title {
            align-self: flex-start;
            font-size: 1.3rem;
            font-weight: 600;
            margin-bottom: 20px;
            color: var(--neon-blue);
            text-transform: uppercase;
            letter-spacing: 1px;
            display: flex;
            align-items: center;
            gap: 10px;
        }

        /* 64 LED Grid */
        .matrix-grid {
            display: grid;
            grid-template-columns: repeat(8, 1fr);
            gap: 8px;
            width: 100%;
            aspect-ratio: 1;
            max-width: 440px;
            margin: auto;
        }

        .led-cell {
            background-color: #000;
            border-radius: 8px;
            cursor: pointer;
            aspect-ratio: 1;
            transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);
            position: relative;
            border: 1px solid rgba(255, 255, 255, 0.05);
        }

        .led-cell:hover {
            transform: scale(1.12);
            z-index: 10;
            box-shadow: 0 0 15px currentColor;
        }

        .led-cell.active-cursor {
            outline: 2px solid #ffffff;
            outline-offset: 1px;
            animation: pulse-border 1.5s infinite alternate;
        }

        @keyframes pulse-border {
            0% {
                outline-color: rgba(255, 255, 255, 0.4);
                box-shadow: 0 0 4px rgba(255, 255, 255, 0.4);
            }
            100% {
                outline-color: rgba(255, 255, 255, 1);
                box-shadow: 0 0 12px rgba(255, 255, 255, 1);
            }
        }

        /* Color Palette */
        .palette-container {
            display: grid;
            grid-template-columns: repeat(4, 1fr);
            gap: 12px;
            width: 100%;
            margin-bottom: 25px;
        }

        .color-swatch {
            background: rgba(255, 255, 255, 0.03);
            border: 1px solid var(--border-glass);
            border-radius: 16px;
            padding: 10px;
            cursor: pointer;
            display: flex;
            flex-direction: column;
            align-items: center;
            gap: 8px;
            transition: all 0.2s ease;
        }

        .color-swatch:hover {
            background: rgba(255, 255, 255, 0.08);
            transform: translateY(-2px);
        }

        .color-swatch.active {
            border-color: var(--neon-blue);
            background: rgba(0, 210, 255, 0.05);
            box-shadow: 0 0 15px rgba(0, 210, 255, 0.15);
        }

        .color-circle {
            width: 28px;
            height: 28px;
            border-radius: 50%;
            border: 1px solid rgba(255, 255, 255, 0.2);
            box-shadow: 0 4px 10px rgba(0,0,0,0.3);
        }

        .color-name {
            font-size: 0.8rem;
            font-weight: 400;
            color: var(--text-muted);
        }

        .color-swatch.active .color-name {
            color: var(--neon-blue);
            font-weight: 600;
        }

        /* Global Control Buttons */
        .controls-row {
            display: flex;
            gap: 12px;
            width: 100%;
            margin-bottom: 25px;
        }

        .btn {
            flex: 1;
            background: rgba(255, 255, 255, 0.04);
            border: 1px solid var(--border-glass);
            color: var(--text-main);
            padding: 12px 20px;
            border-radius: 14px;
            font-family: 'Outfit', sans-serif;
            font-weight: 600;
            cursor: pointer;
            transition: all 0.2s ease;
            display: flex;
            align-items: center;
            justify-content: center;
            gap: 8px;
        }

        .btn:hover {
            background: rgba(255, 255, 255, 0.1);
            border-color: rgba(255, 255, 255, 0.2);
            transform: translateY(-1px);
        }

        .btn-primary {
            background: linear-gradient(135deg, #00d2ff 0%, #0087ff 100%);
            border: none;
            color: #fff;
        }

        .btn-primary:hover {
            background: linear-gradient(135deg, #00e0ff 0%, #0097ff 100%);
            box-shadow: 0 0 15px rgba(0, 210, 255, 0.3);
        }

        /* Relays Panel */
        .relays-grid {
            display: grid;
            grid-template-columns: repeat(4, 1fr);
            gap: 12px;
            width: 100%;
        }

        .relay-card {
            background: rgba(255, 255, 255, 0.02);
            border: 1px solid var(--border-glass);
            border-radius: 18px;
            padding: 15px 10px;
            cursor: pointer;
            display: flex;
            flex-direction: column;
            align-items: center;
            gap: 10px;
            transition: all 0.2s ease;
        }

        .relay-card:hover {
            background: rgba(255, 255, 255, 0.05);
        }

        .relay-card.active {
            border-color: var(--neon-green);
            background: rgba(0, 255, 135, 0.05);
            box-shadow: 0 0 15px rgba(0, 255, 135, 0.15);
        }

        .relay-status {
            width: 10px;
            height: 10px;
            border-radius: 50%;
            background-color: #3f4e60;
            transition: all 0.2s ease;
        }

        .relay-card.active .relay-status {
            background-color: var(--neon-green);
            box-shadow: 0 0 8px var(--neon-green);
        }

        .relay-label {
            font-size: 0.85rem;
            font-weight: 600;
            color: var(--text-muted);
        }

        .relay-card.active .relay-label {
            color: var(--neon-green);
        }

        /* Status bar */
        .status-bar {
            margin-top: 30px;
            font-size: 0.9rem;
            color: var(--text-muted);
            display: flex;
            align-items: center;
            gap: 8px;
            background: rgba(255,255,255,0.02);
            padding: 8px 16px;
            border-radius: 20px;
            border: 1px solid var(--border-glass);
        }

        .status-dot {
            width: 8px;
            height: 8px;
            border-radius: 50%;
            background-color: #ff3b30;
            animation: pulse-dot 1.5s infinite alternate;
        }

        .status-dot.connected {
            background-color: var(--neon-green);
            box-shadow: 0 0 8px var(--neon-green);
        }

        @keyframes pulse-dot {
            0% { opacity: 0.4; }
            100% { opacity: 1; }
        }
    </style>
</head>
<body>
    <div class="title-container">
        <h1>NeuraSync LED Interface</h1>
        <div class="subtitle">Bi-Directional Pi 5 Hardware Control Suite</div>
    </div>

    <div class="dashboard">
        <!-- Matrix Panel -->
        <div class="panel">
            <div class="panel-title">
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2" ry="2"></rect><line x1="9" y1="3" x2="9" y2="21"></line><line x1="15" y1="3" x2="15" y2="21"></line><line x1="3" y1="9" x2="21" y2="9"></line><line x1="3" y1="15" x2="21" y2="15"></line></svg>
                64-LED NeoPixel Matrix
            </div>
            <div class="matrix-grid" id="matrix"></div>
        </div>

        <!-- Controls Panel -->
        <div class="panel">
            <div class="panel-title">
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9"></path><path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4L16.5 3.5z"></path></svg>
                Palette & Hardware
            </div>

            <!-- Color Palette Selector -->
            <div class="palette-container" id="palette"></div>

            <!-- Controls Row -->
            <div class="controls-row">
                <button class="btn btn-primary" onclick="fillAll()">
                    Fill Strip
                </button>
                <button class="btn" onclick="clearAll()">
                    Black Out
                </button>
            </div>

            <!-- Relay Row -->
            <div class="panel-title" style="margin-top: 15px;">
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"></polygon></svg>
                MCP23017 Relays
            </div>
            <div class="relays-grid" id="relays"></div>
        </div>
    </div>

    <div class="status-bar">
        <div class="status-dot" id="status-dot"></div>
        <span id="status-text">Connecting to Pi...</span>
    </div>

    <script>
        const colorPalette = [
            [0, 0, 150],    // Blue (Startup default)
            [150, 0, 0],    // Red
            [0, 150, 0],    // Green
            [150, 120, 0],  // Yellow
            [120, 0, 150],  // Purple
            [0, 150, 150],  // Cyan
            [150, 60, 0],   // Orange
            [120, 120, 120] // White
        ];

        const colorNames = [
            "Blue", "Red", "Green", "Yellow",
            "Purple", "Cyan", "Orange", "White"
        ];

        let activeColorIndex = 0;
        let ws;

        // Initialize UI Elements
        const matrixEl = document.getElementById("matrix");
        const paletteEl = document.getElementById("palette");
        const relaysEl = document.getElementById("relays");
        const statusDot = document.getElementById("status-dot");
        const statusText = document.getElementById("status-text");

        // 1. Create Grid cells
        for (let i = 0; i < 64; i++) {
            const cell = document.createElement("div");
            cell.className = "led-cell";
            cell.id = `cell-${i}`;
            cell.addEventListener("click", () => {
                sendPaint(i);
            });
            matrixEl.appendChild(cell);
        }

        // 2. Create Swatches
        colorPalette.forEach((rgb, idx) => {
            const swatch = document.createElement("div");
            swatch.className = `color-swatch ${idx === 0 ? 'active' : ''}`;
            swatch.id = `swatch-${idx}`;
            
            const circle = document.createElement("div");
            circle.className = "color-circle";
            circle.style.backgroundColor = `rgb(${rgb[0]}, ${rgb[1]}, ${rgb[2]})`;
            circle.style.boxShadow = `0 4px 10px rgba(${rgb[0]}, ${rgb[1]}, ${rgb[2]}, 0.4)`;
            
            const label = document.createElement("div");
            label.className = "color-name";
            label.innerText = colorNames[idx];

            swatch.appendChild(circle);
            swatch.appendChild(label);
            swatch.addEventListener("click", () => {
                selectColor(idx);
            });
            paletteEl.appendChild(swatch);
        });

        // 3. Create Relay cards
        for (let i = 0; i < 8; i++) {
            const card = document.createElement("div");
            card.className = "relay-card";
            card.id = `relay-${i}`;
            
            const dot = document.createElement("div");
            dot.className = "relay-status";
            
            const label = document.createElement("div");
            label.className = "relay-label";
            label.innerText = `Relay ${i + 1}`;

            card.appendChild(dot);
            card.appendChild(label);
            card.addEventListener("click", () => {
                toggleRelay(i);
            });
            relaysEl.appendChild(card);
        }

        // Connection Management
        function connect() {
            const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
            const wsUrl = `${proto}//${window.location.host}/ws`;
            
            statusText.innerText = "Connecting to Pi...";
            statusDot.className = "status-dot";

            ws = new WebSocket(wsUrl);

            ws.onopen = () => {
                statusText.innerText = "System Connected";
                statusDot.className = "status-dot connected";
            };

            ws.onclose = () => {
                statusText.innerText = "Disconnected (Retrying...)";
                statusDot.className = "status-dot";
                setTimeout(connect, 2000);
            };

            ws.onerror = (err) => {
                console.error("WS error: ", err);
                ws.close();
            };

            ws.onmessage = (event) => {
                try {
                    const data = JSON.parse(event.data);
                    if (data.type === "state") {
                        renderState(data);
                    }
                } catch (e) {
                    console.error("Failed to parse socket payload:", e);
                }
            };
        }

        function renderState(state) {
            // Update LEDs
            state.colors.forEach((rgb, idx) => {
                const cell = document.getElementById(`cell-${idx}`);
                if (cell) {
                    cell.style.backgroundColor = `rgb(${rgb[0]}, ${rgb[1]}, ${rgb[2]})`;
                    cell.style.color = `rgb(${rgb[0]}, ${rgb[1]}, ${rgb[2]})`;
                    if (idx === state.cursor) {
                        cell.classList.add("active-cursor");
                    } else {
                        cell.classList.remove("active-cursor");
                    }
                }
            });

            // Update Active Swatch
            activeColorIndex = state.active_color;
            document.querySelectorAll(".color-swatch").forEach((swatch, idx) => {
                if (idx === activeColorIndex) {
                    swatch.classList.add("active");
                } else {
                    swatch.classList.remove("active");
                }
            });

            // Update Relays
            const mask = state.relay_state;
            for (let i = 0; i < 8; i++) {
                const card = document.getElementById(`relay-${i}`);
                if (card) {
                    const active = (mask & (1 << i)) !== 0;
                    if (active) {
                        card.classList.add("active");
                    } else {
                        card.classList.remove("active");
                    }
                }
            }
        }

        // Actions
        function selectColor(idx) {
            activeColorIndex = idx;
            document.querySelectorAll(".color-swatch").forEach((swatch, i) => {
                if (i === idx) swatch.classList.add("active");
                else swatch.classList.remove("active");
            });
            if (ws && ws.readyState === WebSocket.OPEN) {
                ws.send(JSON.stringify({
                    type: "select_color",
                    color_index: idx
                }));
            }
        }

        function sendPaint(index) {
            if (ws && ws.readyState === WebSocket.OPEN) {
                ws.send(JSON.stringify({
                    type: "paint",
                    index: index,
                    color_index: activeColorIndex
                }));
            }
        }

        function toggleRelay(index) {
            if (ws && ws.readyState === WebSocket.OPEN) {
                ws.send(JSON.stringify({
                    type: "toggle_relay",
                    relay_index: index
                }));
            }
        }

        // Global Operations
        function fillAll() {
            if (ws && ws.readyState === WebSocket.OPEN) {
                ws.send(JSON.stringify({
                    type: "fill_all",
                    color_index: activeColorIndex
                }));
            }
        }

        function clearAll() {
            if (ws && ws.readyState === WebSocket.OPEN) {
                ws.send(JSON.stringify({
                    type: "clear_all"
                }));
            }
        }

        // Run
        connect();
    </script>
</body>
</html>
"#;
