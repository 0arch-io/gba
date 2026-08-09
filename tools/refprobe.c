// Reference probe: run any ROM in libmgba with the SAME scripted input as the
// Rust emulator (GBA_INPUT, same "first-last:key,..." format and the same
// one-frame offset), then dump the frame and the video registers so the two
// can be compared directly.
#include <mgba/core/core.h>
#include <mgba/gba/core.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define MAX_RANGES 512
static struct { int a, b; uint16_t bits; } ranges[MAX_RANGES];
static int range_count;

static uint16_t key_bit(const char* name) {
    if (!strcmp(name, "a")) return 1 << 0;
    if (!strcmp(name, "b")) return 1 << 1;
    if (!strcmp(name, "select")) return 1 << 2;
    if (!strcmp(name, "start")) return 1 << 3;
    if (!strcmp(name, "right")) return 1 << 4;
    if (!strcmp(name, "left")) return 1 << 5;
    if (!strcmp(name, "up")) return 1 << 6;
    if (!strcmp(name, "down")) return 1 << 7;
    if (!strcmp(name, "r")) return 1 << 8;
    if (!strcmp(name, "l")) return 1 << 9;
    return 0;
}

static void parse_script(const char* s) {
    if (!s) return;
    char* copy = strdup(s);
    for (char* part = strtok(copy, ","); part; part = strtok(NULL, ",")) {
        char keyname[32];
        int a, b;
        if (sscanf(part, "%d-%d:%31s", &a, &b, keyname) != 3) continue;
        uint16_t bits = key_bit(keyname);
        if (!bits || range_count >= MAX_RANGES) continue;
        ranges[range_count].a = a;
        ranges[range_count].b = b;
        ranges[range_count].bits = bits;
        range_count++;
    }
    free(copy);
}

int main(int argc, char** argv) {
    if (argc < 3) {
        fprintf(stderr, "usage: refprobe2 <rom> <frames>   (GBA_INPUT holds the script)\n");
        return 1;
    }
    int frames = atoi(argv[2]);
    parse_script(getenv("GBA_INPUT"));
    const char* dump_dir = getenv("GBA_DUMP_DIR");
    const char* dump_every_s = getenv("GBA_DUMP_EVERY");
    int dump_every = dump_every_s ? atoi(dump_every_s) : 0;

    struct mCore* core = GBACoreCreate();
    core->init(core);
    unsigned width, height;
    core->desiredVideoDimensions(core, &width, &height);
    uint32_t* vbuf = malloc(width * height * 4);
    core->setVideoBuffer(core, vbuf, width);
    mCoreLoadFile(core, argv[1]);
    mCoreConfigInit(&core->config, "refprobe2");
    core->reset(core);

    char path[512];
    for (int f = 1; f <= frames; ++f) {
        // The Rust runner computes the held keys from the count of COMPLETED
        // frames, so the keys seen during frame f come from f-1. Match that.
        int n = f - 1;
        uint16_t keys = 0;
        for (int i = 0; i < range_count; ++i) {
            if (n >= ranges[i].a && n < ranges[i].b) keys |= ranges[i].bits;
        }
        core->setKeys(core, keys);
        core->runFrame(core);

        if (dump_every > 0 && dump_dir && f % dump_every == 0) {
            snprintf(path, sizeof path, "%s/frame%05d.ppm", dump_dir, f);
            FILE* out = fopen(path, "w");
            if (out) {
                fprintf(out, "P3\n%u %u\n255\n", width, height);
                for (unsigned i = 0; i < width * height; ++i) {
                    uint32_t p = vbuf[i];
                    fprintf(out, "%u %u %u\n", p & 0xFF, (p >> 8) & 0xFF, (p >> 16) & 0xFF);
                }
                fclose(out);
            }
        }
    }

    printf("DISPCNT=%04X BG0CNT=%04X BG1CNT=%04X BG2CNT=%04X BG3CNT=%04X\n",
           core->busRead16(core, 0x04000000), core->busRead16(core, 0x04000008),
           core->busRead16(core, 0x0400000A), core->busRead16(core, 0x0400000C),
           core->busRead16(core, 0x0400000E));
    printf("BLDCNT=%04X BLDALPHA=%04X BLDY=%04X MOSAIC=%04X\n",
           core->busRead16(core, 0x04000050), core->busRead16(core, 0x04000052),
           core->busRead16(core, 0x04000054), core->busRead16(core, 0x0400004C));
    printf("WIN0H=%04X WIN1H=%04X WIN0V=%04X WIN1V=%04X WININ=%04X WINOUT=%04X\n",
           core->busRead16(core, 0x04000040), core->busRead16(core, 0x04000042),
           core->busRead16(core, 0x04000044), core->busRead16(core, 0x04000046),
           core->busRead16(core, 0x04000048), core->busRead16(core, 0x0400004A));
    printf("BG0HOFS=%04X BG0VOFS=%04X BG1HOFS=%04X BG1VOFS=%04X\n",
           core->busRead16(core, 0x04000010), core->busRead16(core, 0x04000012),
           core->busRead16(core, 0x04000014), core->busRead16(core, 0x04000016));

    // Dump the video memories so they can be diffed against the Rust core's.
    FILE* vf = fopen("ref_vram.bin", "wb");
    for (uint32_t a = 0; a < 0x18000; ++a) fputc(core->busRead8(core, 0x06000000 + a), vf);
    fclose(vf);
    FILE* pf = fopen("ref_pal.bin", "wb");
    for (uint32_t a = 0; a < 0x400; ++a) fputc(core->busRead8(core, 0x05000000 + a), pf);
    fclose(pf);
    FILE* of = fopen("ref_oam.bin", "wb");
    for (uint32_t a = 0; a < 0x400; ++a) fputc(core->busRead8(core, 0x07000000 + a), of);
    fclose(of);

    FILE* out = fopen("ref_frame.ppm", "w");
    fprintf(out, "P3\n%u %u\n255\n", width, height);
    for (unsigned i = 0; i < width * height; ++i) {
        uint32_t p = vbuf[i];
        fprintf(out, "%u %u %u\n", p & 0xFF, (p >> 8) & 0xFF, (p >> 16) & 0xFF);
    }
    fclose(out);
    return 0;
}
