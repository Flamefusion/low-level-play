use i2cdev::core::I2CDevice;
use i2cdev::linux::LinuxI2CDevice;

// Registers
const IODIRA: u8 = 0x00;
const IODIRB: u8 = 0x01;
const GPIOA: u8 = 0x12;
const GPIOB: u8 = 0x13;

pub struct MCPController {
    bus_number: u8,
    address: u16,
    device: LinuxI2CDevice,
}

impl MCPController {
    /// Creates a new MCP controller on the specified I2C bus and address
    pub fn new(bus_number: u8, address: u16) -> Result<Self, Box<dyn std::error::Error>> {
        let bus_path = format!("/dev/i2c-{}", bus_number);
        log::info!("Opening I2C device on {} at address 0x{:02X}", bus_path, address);
        let device = LinuxI2CDevice::new(&bus_path, address)?;

        Ok(Self { bus_number, address, device })
    }

    /// Initializes Port A as outputs (relays) and Port B as inputs (buttons)
    pub fn initialize(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        log::info!("Initializing MCP23017 registers (Port A -> Out, Port B -> In)...");
        // Write 0x00 to IODIRA -> Set all pins on Port A as OUTPUT
        self.device.smbus_write_byte_data(IODIRA, 0x00)?;

        // Write 0xFF to IODIRB -> Set all pins on Port B as INPUT
        self.device.smbus_write_byte_data(IODIRB, 0xFF)?;

        log::debug!("MCP23017 on bus {} address 0x{:02X} successfully initialized.", self.bus_number, self.address);
        Ok(())
    }

    /// Sets the state of all relays on Port A (each bit represents a relay: 1 = ON, 0 = OFF)
    pub fn set_relays(&mut self, state_mask: u8) -> Result<(), Box<dyn std::error::Error>> {
        log::debug!("Writing relay state mask: 0b{:08b}", state_mask);
        self.device.smbus_write_byte_data(GPIOA, state_mask)?;
        Ok(())
    }

    /// Reads button states from Port B
    /// Each bit representing GPB0-GPB7: 1 means signal HIGH, 0 means signal LOW
    pub fn read_buttons(&mut self) -> Result<u8, Box<dyn std::error::Error>> {
        let buttons = self.device.smbus_read_byte_data(GPIOB)?;
        log::trace!("Read button state mask: 0b{:08b}", buttons);
        Ok(buttons)
    }
}
