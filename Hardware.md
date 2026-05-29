## Physical Hardware Architecture

```
Raspberry Pi 5
│
├── I²C Bus (auto-detected: bus 1, 13, or 14)
│   ├── MCP23017 #1  addr=0x20  → Slots  1–8  (Port A = relay OUT, Port B = button IN)
│   ├── MCP23017 #2  addr=0x21  → Slots  9–16
│   ├── MCP23017 #3  addr=0x22  → Slots 17–24
│   ├── MCP23017 #4  addr=0x23  → Slots 25–32
│   ├── MCP23017 #5  addr=0x24  → Slots 33–40
│   ├── MCP23017 #6  addr=0x25  → Slots 41–48
│   ├── MCP23017 #7  addr=0x26  → Slots 49–56
│   └── MCP23017 #8  addr=0x27  → Slots 57–64
│
├── GPIO (lgpio, gpiochip0)
│   ├── GPIO  5 → RESET# for MCP #1
│   ├── GPIO  6 → RESET# for MCP #2
│   ├── GPIO 13 → RESET# for MCP #3
│   └── GPIO 12 → RESET# for MCP #4
│
├── SPI / UART (device "/dev/spidev0.0" assumed)
│   └── Pi5Neo LED strip — 64 RGB WS2812B/SK6812 LEDs
│
└── USB
    └── TP-Link Bluetooth USB Adapter → hci1 (BLE 4.0/5.0)
```

The Pi uses **SMBus** (`smbus` Python library, wrapping the Linux `/dev/i2c-N` device). An auto-detection routine tries I²C buses 1, 13, 14 in order and picks the first one where all 8 MCP addresses respond.

---

## 3. MCP23017 — I2C I/O Expanders

### 3.1 Register Map (used registers only)

| Register | Address | Description |
|----------|---------|-------------|
| IODIRA   | 0x00    | I/O Direction A — `0x00` = all outputs (charger relays) |
| IODIRB   | 0x01    | I/O Direction B — `0xFF` = all inputs (buttons) |
| GPIOA    | 0x12    | Read/Write Port A GPIO pins (charger side) |
| GPIOB    | 0x13    | Read-only Port B GPIO pins (button side) |
| OLATA    | 0x14    | Output Latch A — used for R-M-W charger control |
| OLATB    | 0x15    | Output Latch B (not used) |

### 3.2 Initialisation Sequence

Performed once at import time (module-level `MCPController.setup_all_mcps()`):

```
1. Open gpiochip0 via lgpio
2. For each reset pin in {GPIO5, GPIO6, GPIO13, GPIO12}:
   - claim as output
   - write HIGH (deassert reset)
3. For each MCP address 0x20–0x27:
   - i2c_write(addr, IODIRA, 0x00)   # Port A = all output
   - sleep 10ms
   - i2c_write(addr, IODIRB, 0xFF)   # Port B = all input
   - sleep 50ms
```

### 3.3 Location-to-MCP-to-Pin Mapping

There are two configurable modes set by `BUTTON_MAPPING_MODE` env var:

**DIRECT mode** (current production):
```
Location N → MCP at address (0x20 + floor((N-1)/8))
           → Pin  ((N-1) % 8) + 1      (1-indexed)
           → Bit   (N-1) % 8           (0-indexed, for register access)

Example: Location 17 → MCP 0x22 (MCP #3), pin 1, bit 0
```
