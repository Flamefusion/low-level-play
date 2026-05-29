mod gpio;
mod mcp;
mod neo;

use mcp::MCPController;
use neo::NeoPixelStrip;
use std::thread::sleep;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("--- 🦀 Low-Level Rust Hardware Control ---");

    // 1. Release hardware resets via GPIO
    if let Err(e) = gpio::initialize_mcp_resets() {
        eprintln!(
            "Failed to reset MCP chips: {}. Ensure you are running as sudo or have gpio permissions.",
            e
        );
        return Err(e);
    }

    // 2. Initialize NeoPixel SPI strip (64 LEDs)
    println!("Initializing NeoPixel LED strip...");
    let mut strip = match NeoPixelStrip::new(64) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!(
                "[Warning] Failed to initialize NeoPixel LED strip: {}.\n\
                 Ensure SPI is enabled on your Raspberry Pi (via sudo raspi-config or dtparam=spi=on in config.txt).\n\
                 Proceeding with I2C MCP23017 relay control loop...",
                e
            );
            None
        }
    };

    // 3. Initialize MCP chip #1 (Address 0x20) on Bus 1 (or 13/14, adapt based on auto-detection)
    let bus_number = 1;
    let mcp_address = 0x20;
    println!(
        "Connecting to MCP23017 #1 at bus {} addr 0x{:02X}...",
        bus_number, mcp_address
    );
    let mut mcp = MCPController::new(bus_number, mcp_address)?;
    mcp.initialize()?;

    // 4. Create an elegant LED color cycle (red, green, blue patterns)
    if let Some(ref mut s) = strip {
        println!("Illuminating NeoPixel LED strip sequentially...");
        let mut led_colors = vec![(0, 0, 0); 64];

        for i in 0..64 {
            // Draw a pattern of repeating Red, Green, Blue on the LED strip
            led_colors[i] = match i % 3 {
                0 => (30, 0, 0), // Soft Red
                1 => (0, 30, 0), // Soft Green
                _ => (0, 0, 30), // Soft Blue
            };
            s.show(&led_colors)?;
            // 20ms pause for a smooth animated wipe-on effect
            sleep(Duration::from_millis(20));
        }
        println!("LED strip fully illuminated!");
    }

    // 5. Test MCP23017 Relays and Buttons in a short loop
    println!("\nStarting hardware control loop. Press Ctrl+C to exit.");
    let mut relay_state = 0x01; // bitmask: start with relay 1 active (00000001)

    for step in 0..20 {
        // Toggle the active relay byte
        mcp.set_relays(relay_state)?;

        // Read input buttons
        let buttons = mcp.read_buttons()?;

        // Print progress
        println!(
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
            s.show(&led_colors)?;
        }

        // Rotate the bit so next relay turns on
        relay_state = relay_state.rotate_left(1);

        sleep(Duration::from_millis(500));
    }

    // Clean up: turn off relays and clear LEDs when exiting
    println!("\nShutting down hardware cleanly...");
    mcp.set_relays(0x00)?;

    if let Some(ref mut s) = strip {
        println!("Extinguishing NeoPixels sequentially...");
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
            s.show(&current_colors)?;
            sleep(Duration::from_millis(15));
        }
    }

    println!("Goodbye!");

    Ok(())
}
