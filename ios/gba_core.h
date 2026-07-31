#ifndef GBA_CORE_H
#define GBA_CORE_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

void* gba_create(const uint8_t* rom, size_t rom_len, const uint8_t* sav, size_t sav_len);
void gba_destroy(void* h);
void gba_run_frame(void* h, uint16_t keys);
const uint32_t* gba_framebuffer(void* h);
size_t gba_audio_read(void* h, float* out, size_t max);
bool gba_flash_dirty(void* h);
size_t gba_flash_read(void* h, uint8_t* out, size_t max);
size_t gba_state_save(void* h, uint8_t* out, size_t max);
bool gba_state_load(void* h, const uint8_t* data, size_t len);

#endif
