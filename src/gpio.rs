use gpiod::{Chip, Options};
use std::thread::sleep;
use std::time::Duration;

/// Deasserts (sets HIGH) the hardware RESET pins on the MCP chips and returns the lines handle
pub fn initialize_mcp_resets() -> Result<gpiod::Lines<gpiod::Output>, Box<dyn std::error::Error>> {
    println!("Initializing MCP hardware resets...");

    // Open gpiochip0 (the primary RP1 GPIO expander on Pi 5)
    let chip = Chip::new("/dev/gpiochip0")?;

    // Request GPIO lines 5, 6, 12, 13 as outputs
    let options = Options::output([5, 6, 12, 13]).consumer("mcp-reset");

    let requested = chip.request_lines(options)?;

    // Pull reset LOW for 10 milliseconds to force a physical hardware reset
    requested.set_values([false, false, false, false])?;
    sleep(Duration::from_millis(10));

    // Pull reset HIGH to release the MCP chips and boot them up!
    requested.set_values([true, true, true, true])?;
    println!("MCP Chips successfully pulled out of Hardware Reset!");

    Ok(requested)
}
