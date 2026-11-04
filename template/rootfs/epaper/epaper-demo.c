/*
 * Waveshare 2.7inch e-Paper HAT demo for the Luckfox Lyra Zero W.
 *
 * The HAT seats directly on the Lyra header: all 12 Raspberry Pi power and
 * ground pins line up. What does not line up is the SPI pinmux: the Pi puts
 * MOSI on pin 19 and CE0 on pin 24, the Lyra's SPI1 puts them elsewhere. So
 * the default is --bitbang, which clocks the panel from userspace GPIO. All
 * seven pins are unclaimed in the stock device tree, so this needs no
 * boot.img change and cannot stop the board booting.
 *
 * To use hardware SPI instead, move SPI1 onto these pins in the device tree
 * and rebuild boot.img from the Luckfox SDK. Do not repack boot.img by hand.
 *
 *   HAT    Pi pin  Lyra pin   Linux GPIO  driven by
 *   VCC    1       3V3_OUT    -           -
 *   GND    6       GND        -           -
 *   RST    11      GPIO0_A3   3           sysfs GPIO
 *   PWR    12      GPIO0_B6   14          sysfs GPIO, held high
 *   BUSY   18      GPIO0_B4   12          sysfs GPIO, input
 *   DIN    19      GPIO0_A6   6           bit-banged, or SPI1_MOSI
 *   DC     22      GPIO0_B3   11          sysfs GPIO
 *   CLK    23      GPIO0_B0   8           bit-banged, or SPI1_CLK
 *   CS     24      GPIO0_B2   10          bit-banged, or SPI1_CSN0
 *
 * Two panel revisions exist and they are not compatible. V1 is a UC8151-class
 * controller programmed with power/booster/PLL commands and five waveform
 * LUTs. V2 is an SSD1675-class controller programmed with RAM windows and an
 * update sequence. Their BUSY pins are even opposite polarity: V1 is busy
 * when BUSY is low, V2 when it is high. The demo tries one, and if the panel
 * never answers, tries the other.
 *
 * Panel command sequences are Waveshare's, from EPD_2in7.c and EPD_2in7_V2.c.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <linux/spi/spidev.h>
#include <signal.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <time.h>
#include <unistd.h>

#define EPD_W 176
#define EPD_H 264
#define ROW_BYTES (EPD_W / 8)        /* 22    */
#define FB_SIZE (ROW_BYTES * EPD_H)  /* 5808  */

#define PIN_RST 3
#define PIN_PWR 14
#define PIN_BUSY 12
#define PIN_DC 11

#define SPI_DEV "/dev/spidev1.0"
#define SPI_HZ 4000000
#define BUSY_TIMEOUT_MS 15000

static int spi_fd = -1, dc_fd = -1, rst_fd = -1, pwr_fd = -1, busy_fd = -1;
static int panel_v2 = 1;     /* default V2; -1 forces V1 */
static int use_bitbang = 0;
static volatile sig_atomic_t stop_now = 0;

static void on_signal(int s) { (void)s; stop_now = 1; }

static void msleep(long ms)
{
    struct timespec ts = { ms / 1000, (ms % 1000) * 1000000L };
    nanosleep(&ts, NULL);
}

static void logmsg(const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    vfprintf(stderr, fmt, ap);
    va_end(ap);
    fputc('\n', stderr);
    fflush(stderr);
}

/* ---- sysfs GPIO ---- */

static int write_file(const char *path, const char *val)
{
    int fd = open(path, O_WRONLY);
    if (fd < 0)
        return -1;
    ssize_t n = write(fd, val, strlen(val));
    close(fd);
    return n < 0 ? -1 : 0;
}

static int gpio_setup(int gpio, const char *dir)
{
    char buf[32], path[96];
    snprintf(buf, sizeof buf, "%d", gpio);
    write_file("/sys/class/gpio/export", buf); /* EBUSY if already there */
    snprintf(path, sizeof path, "/sys/class/gpio/gpio%d/direction", gpio);
    for (int i = 0; i < 50; i++) { /* udev may not have fixed the mode yet */
        if (write_file(path, dir) == 0)
            return 0;
        msleep(20);
    }
    logmsg("gpio %d: cannot set direction to %s", gpio, dir);
    return -1;
}

static int gpio_value_fd(int gpio, int flags)
{
    char path[96];
    snprintf(path, sizeof path, "/sys/class/gpio/gpio%d/value", gpio);
    int fd = open(path, flags);
    if (fd < 0)
        logmsg("gpio %d: cannot open value: %s", gpio, strerror(errno));
    return fd;
}

static void gpio_set(int fd, int v)
{
    if (fd >= 0)
        (void)!write(fd, v ? "1" : "0", 1);
}

static int gpio_get(int fd)
{
    char c = '0';
    if (fd < 0)
        return -1;
    lseek(fd, 0, SEEK_SET);
    if (read(fd, &c, 1) != 1)
        return -1;
    return c == '1';
}

/* ---- transport: hardware spidev, or bit-bang on the same pins ---- */

#define BB_CLK 8
#define BB_MOSI 6
#define BB_CS 10
static int bb_clk_fd = -1, bb_mosi_fd = -1, bb_cs_fd = -1;

static int spi_open(void)
{
    uint8_t mode = SPI_MODE_0, bits = 8;
    uint32_t hz = SPI_HZ;
    int fd = open(SPI_DEV, O_RDWR);
    if (fd < 0) {
        logmsg("cannot open %s: %s", SPI_DEV, strerror(errno));
        return -1;
    }
    if (ioctl(fd, SPI_IOC_WR_MODE, &mode) < 0
        || ioctl(fd, SPI_IOC_WR_BITS_PER_WORD, &bits) < 0
        || ioctl(fd, SPI_IOC_WR_MAX_SPEED_HZ, &hz) < 0) {
        logmsg("%s: ioctl failed: %s", SPI_DEV, strerror(errno));
        close(fd);
        return -1;
    }
    return fd;
}

static int bitbang_open(void)
{
    if (gpio_setup(BB_CLK, "out") || gpio_setup(BB_MOSI, "out")
        || gpio_setup(BB_CS, "out"))
        return -1;
    bb_clk_fd = gpio_value_fd(BB_CLK, O_WRONLY);
    bb_mosi_fd = gpio_value_fd(BB_MOSI, O_WRONLY);
    bb_cs_fd = gpio_value_fd(BB_CS, O_WRONLY);
    if (bb_clk_fd < 0 || bb_mosi_fd < 0 || bb_cs_fd < 0)
        return -1;
    gpio_set(bb_cs_fd, 1);
    gpio_set(bb_clk_fd, 0);
    return 0;
}

/* Mode 0: data is set while the clock is low and sampled on the rising edge. */
static void bitbang_write(const uint8_t *buf, size_t len)
{
    gpio_set(bb_cs_fd, 0);
    for (size_t i = 0; i < len; i++) {
        for (int b = 7; b >= 0; b--) {
            gpio_set(bb_clk_fd, 0);
            gpio_set(bb_mosi_fd, (buf[i] >> b) & 1);
            gpio_set(bb_clk_fd, 1);
        }
    }
    gpio_set(bb_clk_fd, 0);
    gpio_set(bb_cs_fd, 1);
}

static void xfer(const uint8_t *buf, size_t len)
{
    if (use_bitbang) {
        bitbang_write(buf, len);
        return;
    }
    while (len) {
        size_t chunk = len > 4096 ? 4096 : len;
        struct spi_ioc_transfer tr;
        memset(&tr, 0, sizeof tr);
        tr.tx_buf = (unsigned long)buf;
        tr.len = chunk;
        tr.speed_hz = SPI_HZ;
        tr.bits_per_word = 8;
        if (ioctl(spi_fd, SPI_IOC_MESSAGE(1), &tr) < 0) {
            logmsg("spi transfer failed: %s", strerror(errno));
            return;
        }
        buf += chunk;
        len -= chunk;
    }
}

static void cmd(uint8_t c)
{
    gpio_set(dc_fd, 0);
    xfer(&c, 1);
}

static void dat(uint8_t d)
{
    gpio_set(dc_fd, 1);
    xfer(&d, 1);
}

static void dat_block(const uint8_t *p, size_t n)
{
    gpio_set(dc_fd, 1);
    xfer(p, n);
}

/* Returns 0 when the panel went idle, -1 on timeout. V1 is busy while BUSY
 * reads low; V2 is busy while it reads high. */
static int wait_idle(void)
{
    int waited = 0;
    for (;;) {
        if (panel_v2) {
            if (gpio_get(busy_fd) == 0)
                return 0;
        } else {
            cmd(0x71);
            if (gpio_get(busy_fd) == 1)
                return 0;
        }
        if (waited >= BUSY_TIMEOUT_MS)
            return -1;
        msleep(20);
        waited += 20;
        if (stop_now)
            return -1;
    }
}

static void panel_reset(void)
{
    int hold = panel_v2 ? 20 : 200;
    gpio_set(rst_fd, 1);
    msleep(hold);
    gpio_set(rst_fd, 0);
    msleep(5);
    gpio_set(rst_fd, 1);
    msleep(hold);
}

/* ---- panel init, both revisions ---- */

static const uint8_t lut_vcom_dc[] = {
    0x00,0x00,0x00,0x08,0x00,0x00,0x00,0x02,0x60,0x28,0x28,0x00,0x00,0x01,
    0x00,0x14,0x00,0x00,0x00,0x01,0x00,0x12,0x12,0x00,0x00,0x01,0x00,0x00,
    0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,
    0x00,0x00
};
static const uint8_t lut_ww[] = {
    0x40,0x08,0x00,0x00,0x00,0x02,0x90,0x28,0x28,0x00,0x00,0x01,0x40,0x14,
    0x00,0x00,0x00,0x01,0xA0,0x12,0x12,0x00,0x00,0x01,0x00,0x00,0x00,0x00,
    0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00
};
static const uint8_t lut_bb[] = {
    0x80,0x08,0x00,0x00,0x00,0x02,0x90,0x28,0x28,0x00,0x00,0x01,0x80,0x14,
    0x00,0x00,0x00,0x01,0x50,0x12,0x12,0x00,0x00,0x01,0x00,0x00,0x00,0x00,
    0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00
};

static void send_lut(uint8_t reg, const uint8_t *p, size_t n)
{
    cmd(reg);
    for (size_t i = 0; i < n; i++)
        dat(p[i]);
}

static int init_v1(void)
{
    static const uint8_t power_opt[][2] = {
        { 0x60, 0xA5 }, { 0x89, 0xA5 }, { 0x90, 0x00 }, { 0x93, 0x2A },
        { 0xA0, 0xA5 }, { 0xA1, 0x00 }, { 0x73, 0x41 },
    };
    panel_reset();

    cmd(0x01); dat(0x03); dat(0x00); dat(0x2b); dat(0x2b); dat(0x09);
    cmd(0x06); dat(0x07); dat(0x07); dat(0x17);
    for (size_t i = 0; i < sizeof power_opt / sizeof power_opt[0]; i++) {
        cmd(0xF8); dat(power_opt[i][0]); dat(power_opt[i][1]);
    }
    cmd(0x16); dat(0x00);
    cmd(0x04);                       /* power on */
    if (wait_idle() < 0)
        return -1;
    cmd(0x00); dat(0xAF);            /* panel setting */
    cmd(0x30); dat(0x3A);            /* PLL, 100 Hz */
    cmd(0x82); dat(0x12);            /* VCM DC */

    send_lut(0x20, lut_vcom_dc, sizeof lut_vcom_dc);
    send_lut(0x21, lut_ww, sizeof lut_ww);
    send_lut(0x22, lut_ww, sizeof lut_ww);  /* bw uses the ww waveform */
    send_lut(0x23, lut_bb, sizeof lut_bb);
    send_lut(0x24, lut_bb, sizeof lut_bb);  /* wb uses the bb waveform */
    return 0;
}

static int init_v2(void)
{
    panel_reset();
    if (wait_idle() < 0)
        return -1;
    cmd(0x12);                       /* software reset */
    if (wait_idle() < 0)
        return -1;
    cmd(0x45); dat(0x00); dat(0x00); dat(0x07); dat(0x01); /* RAM Y 0..263 */
    cmd(0x4F); dat(0x00); dat(0x00);                       /* RAM Y counter */
    cmd(0x11); dat(0x03);                                  /* data entry */
    return 0;
}

static void display(const uint8_t *fb)
{
    if (panel_v2) {
        cmd(0x24);
        dat_block(fb, FB_SIZE);
        cmd(0x22); dat(0xF7);
        cmd(0x20);
    } else {
        cmd(0x13);
        dat_block(fb, FB_SIZE);
        cmd(0x12);
    }
    wait_idle();
}

static void panel_sleep(void)
{
    if (panel_v2) {
        cmd(0x10); dat(0x01);
    } else {
        cmd(0x50); dat(0xf7);
        cmd(0x02);
        cmd(0x07); dat(0xA5);
    }
}

/* ---- framebuffer ---- */
/* One bit per pixel, 1 is white. Byte 0 holds the leftmost 8 pixels of row 0,
 * with the leftmost pixel in bit 7. */

static uint8_t fb[FB_SIZE];

static void fb_clear(int white) { memset(fb, white ? 0xFF : 0x00, FB_SIZE); }

static void px(int x, int y, int black)
{
    if (x < 0 || y < 0 || x >= EPD_W || y >= EPD_H)
        return;
    uint8_t *p = &fb[y * ROW_BYTES + (x >> 3)];
    uint8_t m = 0x80 >> (x & 7);
    if (black)
        *p &= (uint8_t)~m;
    else
        *p |= m;
}

static void fill(int x, int y, int w, int h, int black)
{
    for (int j = y; j < y + h; j++)
        for (int i = x; i < x + w; i++)
            px(i, j, black);
}

static void frame(int x, int y, int w, int h, int t, int black)
{
    fill(x, y, w, t, black);
    fill(x, y + h - t, w, t, black);
    fill(x, y, t, h, black);
    fill(x + w - t, y, t, h, black);
}

/* 5x7 font, ASCII 32..90. Each glyph is five column bytes, bit 0 is the top
 * row. Enough for upper case, digits and a little punctuation. */
static const uint8_t font5x7[][5] = {
    {0,0,0,0,0},{0,0,0x5F,0,0},{0,7,0,7,0},{0x14,0x7F,0x14,0x7F,0x14},
    {0x24,0x2A,0x7F,0x2A,0x12},{0x23,0x13,8,0x64,0x62},{0x36,0x49,0x55,0x22,0x50},
    {0,5,3,0,0},{0,0x1C,0x22,0x41,0},{0,0x41,0x22,0x1C,0},{0x14,8,0x3E,8,0x14},
    {8,8,0x3E,8,8},{0,0x50,0x30,0,0},{8,8,8,8,8},{0,0x60,0x60,0,0},
    {0x20,0x10,8,4,2},
    {0x3E,0x51,0x49,0x45,0x3E},{0,0x42,0x7F,0x40,0},{0x42,0x61,0x51,0x49,0x46},
    {0x21,0x41,0x45,0x4B,0x31},{0x18,0x14,0x12,0x7F,0x10},{0x27,0x45,0x45,0x45,0x39},
    {0x3C,0x4A,0x49,0x49,0x30},{1,0x71,9,5,3},{0x36,0x49,0x49,0x49,0x36},
    {6,0x49,0x49,0x29,0x1E},
    {0,0x36,0x36,0,0},{0,0x56,0x36,0,0},{8,0x14,0x22,0x41,0},
    {0x14,0x14,0x14,0x14,0x14},{0,0x41,0x22,0x14,8},{2,1,0x51,9,6},
    {0x32,0x49,0x79,0x41,0x3E},
    {0x7E,0x11,0x11,0x11,0x7E},{0x7F,0x49,0x49,0x49,0x36},{0x3E,0x41,0x41,0x41,0x22},
    {0x7F,0x41,0x41,0x22,0x1C},{0x7F,0x49,0x49,0x49,0x41},{0x7F,9,9,9,1},
    {0x3E,0x41,0x49,0x49,0x7A},{0x7F,8,8,8,0x7F},{0,0x41,0x7F,0x41,0},
    {0x20,0x40,0x41,0x3F,1},{0x7F,8,0x14,0x22,0x41},{0x7F,0x40,0x40,0x40,0x40},
    {0x7F,2,0x0C,2,0x7F},{0x7F,4,8,0x10,0x7F},{0x3E,0x41,0x41,0x41,0x3E},
    {0x7F,9,9,9,6},{0x3E,0x41,0x51,0x21,0x5E},{0x7F,9,0x19,0x29,0x46},
    {0x46,0x49,0x49,0x49,0x31},{1,1,0x7F,1,1},{0x3F,0x40,0x40,0x40,0x3F},
    {0x1F,0x20,0x40,0x20,0x1F},{0x3F,0x40,0x38,0x40,0x3F},{0x63,0x14,8,0x14,0x63},
    {7,8,0x70,8,7},{0x61,0x51,0x49,0x45,0x43},
};

static void draw_char(int x, int y, char c, int scale)
{
    if (c >= 'a' && c <= 'z')
        c = (char)(c - 32);
    if (c < 32 || c > 'Z')
        c = ' ';
    const uint8_t *g = font5x7[c - 32];
    for (int col = 0; col < 5; col++)
        for (int row = 0; row < 7; row++)
            if (g[col] & (1 << row))
                fill(x + col * scale, y + row * scale, scale, scale, 1);
}

static void draw_text(int x, int y, const char *s, int scale)
{
    for (; *s; s++) {
        draw_char(x, y, *s, scale);
        x += 6 * scale;
    }
}

static int text_width(const char *s, int scale)
{
    return (int)strlen(s) * 6 * scale - scale;
}

static void draw_centred(int y, const char *s, int scale)
{
    draw_text((EPD_W - text_width(s, scale)) / 2, y, s, scale);
}

/* ---- the demo itself ---- */

static void splash(void)
{
    fb_clear(1);
    frame(4, 4, EPD_W - 8, EPD_H - 8, 2, 1);
    draw_centred(24, "LUCKFOX", 3);
    draw_centred(56, "LYRA ZERO W", 2);
    draw_centred(84, "RK3506B", 2);
    fill(20, 108, EPD_W - 40, 2, 1);
    draw_centred(124, "2.7 INCH", 2);
    draw_centred(146, "E-PAPER", 2);
    fill(20, 172, EPD_W - 40, 2, 1);
    draw_centred(188, "BIT-BANGED SPI", 1);
    draw_centred(202, "ON PLAIN GPIO", 1);
    draw_centred(224, "HAT SEATED", 1);
    draw_centred(238, "NO JUMPERS", 1);
}

static void bars(int phase)
{
    fb_clear(1);
    frame(4, 4, EPD_W - 8, EPD_H - 8, 2, 1);
    draw_centred(16, "LYRA E-PAPER", 2);
    fill(16, 40, EPD_W - 32, 2, 1);
    for (int i = 0; i < 6; i++) {
        int w = 16 + ((i + phase) % 6) * 22;
        fill(20, 56 + i * 30, w, 18, 1);
    }
    char buf[32];
    snprintf(buf, sizeof buf, "FRAME %d", phase);
    draw_centred(240, buf, 2);
}

static void checker(int phase)
{
    fb_clear(1);
    int s = 22;
    for (int y = 0; y < EPD_H; y += s)
        for (int x = 0; x < EPD_W; x += s)
            if (((x / s) + (y / s) + phase) & 1)
                fill(x, y, s, s, 1);
}

static void cleanup(void)
{
    if (spi_fd >= 0) close(spi_fd);
    if (dc_fd >= 0) close(dc_fd);
    if (rst_fd >= 0) close(rst_fd);
    if (pwr_fd >= 0) close(pwr_fd);
    if (busy_fd >= 0) close(busy_fd);
}

static int bring_up(void)
{
    if (gpio_setup(PIN_RST, "out") || gpio_setup(PIN_DC, "out")
        || gpio_setup(PIN_PWR, "out") || gpio_setup(PIN_BUSY, "in"))
        return -1;
    rst_fd = gpio_value_fd(PIN_RST, O_WRONLY);
    dc_fd = gpio_value_fd(PIN_DC, O_WRONLY);
    pwr_fd = gpio_value_fd(PIN_PWR, O_WRONLY);
    busy_fd = gpio_value_fd(PIN_BUSY, O_RDONLY);
    if (rst_fd < 0 || dc_fd < 0 || pwr_fd < 0 || busy_fd < 0)
        return -1;
    gpio_set(pwr_fd, 1); /* the HAT gates its own supply from this pin */
    msleep(100);

    if (use_bitbang) {
        if (bitbang_open() < 0)
            return -1;
        logmsg("transport: bit-banged SPI on gpio %d/%d/%d", BB_CLK, BB_MOSI, BB_CS);
    } else {
        spi_fd = spi_open();
        if (spi_fd < 0) {
            logmsg("hint: boot.img must be patched to enable spi1, or pass --bitbang");
            return -1;
        }
        logmsg("transport: %s at %d Hz", SPI_DEV, SPI_HZ);
    }
    return 0;
}

int main(int argc, char **argv)
{
    int once = 0;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--v1")) panel_v2 = 0;
        else if (!strcmp(argv[i], "--v2")) panel_v2 = 1;
        else if (!strcmp(argv[i], "--bitbang")) use_bitbang = 1;
        else if (!strcmp(argv[i], "--once")) once = 1;
        else {
            fprintf(stderr,
                "usage: %s [--v1|--v2] [--bitbang] [--once]\n"
                "  --v1       UC8151-class panel (older 2.7inch HAT)\n"
                "  --v2       SSD1675-class panel (default)\n"
                "  --bitbang  drive SPI from GPIO instead of /dev/spidev1.0\n"
                "  --once     draw the splash and exit\n", argv[0]);
            return 2;
        }
    }
    signal(SIGTERM, on_signal);
    signal(SIGINT, on_signal);

    if (bring_up() < 0) {
        cleanup();
        return 1;
    }

    /* Try the selected revision. A panel that never releases BUSY is either
     * absent or the other revision, so try the other once before giving up. */
    int ok = panel_v2 ? init_v2() : init_v1();
    if (ok < 0) {
        logmsg("panel did not respond as %s, trying the other revision",
               panel_v2 ? "V2" : "V1");
        panel_v2 = !panel_v2;
        ok = panel_v2 ? init_v2() : init_v1();
    }
    if (ok < 0) {
        logmsg("no panel responded on either command set; is the HAT seated?");
        cleanup();
        return 1;
    }
    logmsg("panel initialised as %s", panel_v2 ? "V2" : "V1");

    splash();
    display(fb);
    if (once) {
        panel_sleep();
        cleanup();
        return 0;
    }

    /* Full refresh takes seconds and wears the panel, so the loop is slow on
     * purpose: a handful of frames, then a long sleep. */
    int phase = 0;
    while (!stop_now) {
        for (int i = 0; i < 3 && !stop_now; i++) {
            bars(phase + i);
            display(fb);
            sleep(10);
        }
        if (stop_now) break;
        checker(phase);
        display(fb);
        sleep(10);
        splash();
        display(fb);
        phase++;
        for (int i = 0; i < 60 && !stop_now; i++)
            sleep(1);
    }

    logmsg("stopping, putting the panel to sleep");
    panel_sleep();
    cleanup();
    return 0;
}
