use spidev::{SpiModeFlags, Spidev, SpidevOptions};
use std::fs::File;
use std::io::Write;

pub struct NeoPixelStrip {
    spi: Spidev,
    num_leds: usize,
}

impl NeoPixelStrip {
    /// Opens and configures /dev/spidev0.0 for WS2812B control
    pub fn new(num_leds: usize) -> std::io::Result<Self> {
        let mut spi = Spidev::open("/dev/spidev0.0")?;

        // WS2812B expects ~3MHz to 4MHz SPI clock to map bits accurately
        let options = SpidevOptions::new()
            .bits_per_word(8)
            .max_speed_hz(3_000_000)
            .mode(SpiModeFlags::SPI_MODE_0)
            .build();

        spi.configure(&options)?;
        Ok(Self { spi, num_leds })
    }

    /// Renders RGB colors to the LED strip
    /// `colors` is a slice of (r, g, b) tuples, one for each LED
    pub fn show(&mut self, colors: &[(u8, u8, u8)]) -> std::io::Result<()> {
        // WS2812B/SK6812 expects colors in GRB format (Green, Red, Blue)
        let mut spi_buffer = Vec::new();

        for &(r, g, b) in colors.iter().take(self.num_leds) {
            // Encode Green, Red, and Blue bytes
            self.encode_byte(g, &mut spi_buffer);
            self.encode_byte(r, &mut spi_buffer);
            self.encode_byte(b, &mut spi_buffer);
        }

        // The protocol requires a RESET signal (holding data line LOW for >50µs)
        // At 3MHz, 50µs is about 150 bits, which is ~20 empty bytes.
        spi_buffer.resize(spi_buffer.len() + 30, 0x00);

        // Send raw physical signals over the hardware bus!
        self.spi.write_all(&spi_buffer)?;
        self.spi.flush()?;
        Ok(())
    }

    /// Helper to convert a single color byte (8 bits) into 24 SPI bits
    /// Every WS2812B bit is encoded into 3 SPI bits:
    /// '1' -> 0b110
    /// '0' -> 0b100
    fn encode_byte(&self, byte: u8, buffer: &mut Vec<u8>) {
        let mut temp: u32 = 0;

        // We read from the most significant bit (MSB) to least significant (LSB)
        for i in (0..8).rev() {
            let bit = (byte >> i) & 1;
            if bit == 1 {
                // SPI binary '110'
                temp = (temp << 3) | 0b110;
            } else {
                // SPI binary '100'
                temp = (temp << 3) | 0b100;
            }
        }

        // Now `temp` contains 24 bits of SPI data (right-aligned in a 32-bit integer).
        // Let's split this into 3 raw bytes and push to the SPI stream.
        buffer.push(((temp >> 16) & 0xFF) as u8);
        buffer.push(((temp >> 8) & 0xFF) as u8);
        buffer.push((temp & 0xFF) as u8);
    }
}
