mod gpio;
mod mcp;
mod neo;

use mcp::MCPController;
use neo::NeoPixelStrip;
use std::thread::sleep;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize env_logger to support structured log levels (INFO, WARN, DEBUG, TRACE)
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    log::info!("--- 🦀 Low-Level Rust Hardware Control ---");

    // 1. Release hardware resets via GPIO
    if let Err(e) = gpio::initialize_mcp_resets() {
        log::error!(
            "Failed to reset MCP chips: {}. Ensure you are running as sudo or have gpio permissions.",
            e
        );
        return Err(e);
    }

    // 2. Initialize NeoPixel SPI strip (64 LEDs)
    log::info!("Initializing NeoPixel LED strip...");
    let mut strip = match NeoPixelStrip::new(64) {
        Ok(s) => Some(s),
        Err(e) => {
            log::warn!(
                "Failed to initialize NeoPixel LED strip: {}.\n\
                 Ensure SPI is enabled on your Raspberry Pi (via sudo raspi-config or dtparam=spi=on in config.txt).\n\
                 Proceeding with I2C MCP23017 relay control loop...",
                e
            );
            None
        }
    };

    // 3. Initialize MCP chip #1 (Address 0x20) on Bus 1 (with self-healing retry logic)
    let bus_number = 1;
    let mcp_address = 0x20;
    
    let mut mcp = {
        let mut mcp_instance = None;
        let mut attempts = 0;
        let max_attempts = 3;
        
        while attempts < max_attempts {
            attempts += 1;
            log::info!(
                "Connecting to MCP23017 #1 at bus {} addr 0x{:02X} (attempt {}/{})...",
                bus_number, mcp_address, attempts, max_attempts
            );
            
            match MCPController::new(bus_number, mcp_address) {
                Ok(mut controller) => {
                    match controller.initialize() {
                        Ok(_) => {
                            mcp_instance = Some(controller);
                            break;
                        }
                        Err(e) => {
                            log::warn!("MCP controller connected but failed to initialize registers: {}.", e);
                        }
                    }
                }
                Err(e) => {
                    log::warn!("Failed to establish MCP I2C device connection: {}.", e);
                }
            }
            
            if attempts < max_attempts {
                log::info!("Attempting self-healing GPIO hardware reset before retry...");
                let _ = gpio::initialize_mcp_resets();
                sleep(Duration::from_millis(200));
            }
        }
        
        match mcp_instance {
            Some(controller) => controller,
            None => {
                log::error!("Critical: MCP23017 initialization failed after {} attempts.", max_attempts);
                return Err("Failed to initialize MCP controller".into());
            }
        }
    };

    // 4. Create an elegant LED color cycle (red, green, blue patterns)
    if let Some(ref mut s) = strip {
        log::info!("Illuminating NeoPixel LED strip sequentially...");
        let mut led_colors = vec![(0, 0, 0); 64];

        for i in 0..64 {
            // Draw a pattern of repeating Red, Green, Blue on the LED strip
            led_colors[i] = match i % 3 {
                0 => (30, 0, 0), // Soft Red
                1 => (0, 30, 0), // Soft Green
                _ => (0, 0, 30), // Soft Blue
            };
            if let Err(e) = s.show(&led_colors) {
                log::warn!("Failed to write color state to NeoPixel strip at index {}: {}", i, e);
            }
            // 20ms pause for a smooth animated wipe-on effect
            sleep(Duration::from_millis(20));
        }
        log::info!("LED strip fully illuminated!");
    }

    // 5. Test MCP23017 Relays and Buttons in a short loop
    log::info!("Starting hardware control loop. Press Ctrl+C to exit.");
    let mut relay_state = 0x01; // bitmask: start with relay 1 active (00000001)

    for step in 0..20 {
        // Toggle the active relay byte with self-healing retry logic
        let mut attempts = 0;
        let max_attempts = 3;
        loop {
            match mcp.set_relays(relay_state) {
                Ok(_) => break,
                Err(e) => {
                    attempts += 1;
                    log::warn!(
                        "Failed to set relays on attempt {}/{}: {}. Attempting self-healing recovery...",
                        attempts, max_attempts, e
                    );
                    if attempts >= max_attempts {
                        log::error!("Critical: Max self-healing attempts reached. Aborting relay toggle.");
                        return Err(e);
                    }
                    let _ = gpio::initialize_mcp_resets();
                    let _ = mcp.initialize();
                    sleep(Duration::from_millis(100));
                }
            }
        }

        // Read input buttons with self-healing retry logic
        let mut attempts = 0;
        let buttons = loop {
            match mcp.read_buttons() {
                Ok(b) => break b,
                Err(e) => {
                    attempts += 1;
                    log::warn!(
                        "Failed to read buttons on attempt {}/{}: {}. Attempting self-healing recovery...",
                        attempts, max_attempts, e
                    );
                    if attempts >= max_attempts {
                        log::error!("Critical: Max self-healing attempts reached. Aborting button read.");
                        return Err(e);
                    }
                    let _ = gpio::initialize_mcp_resets();
                    let _ = mcp.initialize();
                    sleep(Duration::from_millis(100));
                }
            }
        };

        // Print progress
        log::info!(
            "[Step {:02}] Toggled relays to: 0b{:08b} | Buttons (Port B): 0b{:08b}",
            step, relay_state, buttons
        );

        // Animate the LED strip: shift the color pattern dynamically with each step
        if let Some(ref mut s) = strip {
            let mut led_colors = vec![(0, 0, 0); 64];
            for i in 0..64 {
                led_colors[i] = match (i + step) % 3 {
                    0 => (30, 0, 0), // Soft Red
                    1 => (0, 30, 0), // Soft Green
                    _ => (0, 0, 30), // Soft Blue
                };
            }
            if let Err(e) = s.show(&led_colors) {
                log::warn!("Failed to shift colors on NeoPixel strip during step {}: {}", step, e);
            }
        }

        // Rotate the bit so next relay turns on
        relay_state = relay_state.rotate_left(1);

        sleep(Duration::from_millis(500));
    }

    // Clean up: turn off relays and clear LEDs when exiting
    log::info!("Shutting down hardware cleanly...");
    if let Err(e) = mcp.set_relays(0x00) {
        log::warn!("Failed to disable relays during cleanup: {}", e);
    }

    if let Some(ref mut s) = strip {
        log::info!("Extinguishing NeoPixels sequentially...");
        // Recreate the final color state as a starting point for the wipe-off
        let mut current_colors = vec![(0, 0, 0); 64];
        for i in 0..64 {
            current_colors[i] = match (i + 19) % 3 {
                0 => (30, 0, 0),
                1 => (0, 30, 0),
                _ => (0, 0, 30),
            };
        }
        
        // Turn them off sequentially from the end of the strip to the start
        for i in (0..64).rev() {
            current_colors[i] = (0, 0, 0);
            if let Err(e) = s.show(&current_colors) {
                log::warn!("Failed to clear NeoPixel at index {} during shutdown: {}", i, e);
            }
            sleep(Duration::from_millis(15));
        }
    }

    log::info!("Goodbye!");

    Ok(())
}
