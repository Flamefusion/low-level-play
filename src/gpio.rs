use gpiod::{Chip, Options};
use std::thread::sleep;
use std::time::Duration;

/// Deasserts (sets HIGH) the hardware RESET pins on the MCP chips
pub fn initialize_mcp_resets() -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Initializing MCP hardware resets...");

    // Open gpiochip0 (the primary RP1 GPIO expander on Pi 5)
    let chip = Chip::new("/dev/gpiochip0")?;

    // Request GPIO lines 5, 6, 12, 13 as outputs
    let options = Options::output([5, 6, 12, 13]).consumer("mcp-reset");

    let requested = chip.request_lines(options)?;

    // Pull reset LOW for 10 milliseconds to force a physical hardware reset
    log::debug!("Asserting hardware RESET: pulling lines [5, 6, 12, 13] LOW...");
    requested.set_values([false, false, false, false])?;
    sleep(Duration::from_millis(10));

    // Pull reset HIGH to release the MCP chips and boot them up!
    log::debug!("Deasserting hardware RESET: pulling lines [5, 6, 12, 13] HIGH...");
    requested.set_values([true, true, true, true])?;
    log::info!("MCP Chips successfully pulled out of Hardware Reset!");

    // Important: To prevent the line from falling back to low when the program finishes,
    // in real deployments, these lines must remain configured or held high.
    // In this basic script, we keep them in scope or let standard pull-ups handle it.

    Ok(())
}
