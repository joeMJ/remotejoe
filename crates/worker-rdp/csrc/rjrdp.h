/* Schmale C-Hülle um FreeRDP 3 für den RDP-Worker. */
#ifndef RJRDP_H
#define RJRDP_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct rj_rdp rj_rdp;

typedef struct
{
	void* user;
	/* Bild: gesamter Puffer (BGRA32) und geänderter Bereich. */
	void (*on_frame)(void* user, const uint8_t* data, uint32_t width, uint32_t height,
	                 uint32_t stride, int32_t x, int32_t y, int32_t w, int32_t h);
	void (*on_desktop_resize)(void* user, uint32_t width, uint32_t height);
	/* Zwischenablage (kann aus einem Kanal-Faden kommen). */
	void (*on_clip_ready)(void* user, uint32_t server_flags);
	void (*on_clip_format_list)(void* user, uint32_t n, const uint32_t* ids, const char* const* names);
	void (*on_clip_data_request)(void* user, uint32_t format_id);
	void (*on_clip_data_response)(void* user, int ok, const uint8_t* data, uint32_t len);
	void (*on_clip_file_request)(void* user, uint32_t stream_id, uint32_t list_index, uint32_t flags,
	                             uint64_t position, uint32_t requested);
	void (*on_clip_file_response)(void* user, uint32_t stream_id, int ok, const uint8_t* data,
	                              uint32_t len);
	/* Zertifikat: 1 = annehmen und merken, 2 = nur diesmal, 0 = ablehnen. */
	uint32_t (*verify_certificate)(void* user, const char* host, uint16_t port, const char* subject,
	                               const char* issuer, const char* fingerprint,
	                               const char* old_fingerprint);
	/* Mauszeiger der Gegenstelle: kind 0 = Standardzeiger, 1 = ausgeblendet, 2 = Bild
	 * (BGRA32, w × h, Klickpunkt hx/hy). */
	void (*on_pointer)(void* user, int kind, const uint8_t* bgra, uint32_t w, uint32_t h, uint32_t hx,
	                   uint32_t hy);
} rj_callbacks;

enum rj_security
{
	RJ_SEC_NEGOTIATE = 0,
	RJ_SEC_NLA = 1,
	RJ_SEC_TLS = 2,
	RJ_SEC_RDP = 3
};

typedef struct
{
	const char* host;
	uint16_t port;
	const char* username;
	const char* domain;
	const char* password;
	uint32_t width;
	uint32_t height;
	int security;
	int ignore_certificate;
	int clipboard;
	uint32_t keyboard_layout;
	const char* config_path;
	uint32_t timeout_ms;
	int gfx; /* 1: Grafikkanal (RDPGFX) anbieten */
	int auth_only; /* 1: nur anmelden (NLA), keine Sitzung aufbauen */
} rj_params;

rj_rdp* rj_rdp_new(const rj_callbacks* cb);
void rj_rdp_free(rj_rdp* rdp);
/* 0 = verbunden, sonst Fehler (Text über rj_rdp_last_error). */
int rj_rdp_connect(rj_rdp* rdp, const rj_params* params);
const char* rj_rdp_last_error(rj_rdp* rdp);
/* Grund des Servers für das Verbindungsende (MS-RDPBCGR Set Error Info, z. B. 0x0C Abmeldung). */
uint32_t rj_rdp_error_info(rj_rdp* rdp);
/* Ereignisse verarbeiten; 0 = weiter, -1 = getrennt. */
int rj_rdp_poll(rj_rdp* rdp, uint32_t timeout_ms);
void rj_rdp_disconnect(rj_rdp* rdp);

int rj_rdp_send_unicode(rj_rdp* rdp, uint16_t code, int down);
int rj_rdp_send_scancode(rj_rdp* rdp, uint32_t scancode, int down);
int rj_rdp_send_mouse(rj_rdp* rdp, uint16_t flags, uint16_t x, uint16_t y);
/* Größenänderung über den Anzeige-Steuerkanal; -1 wenn nicht verfügbar. */
int rj_rdp_resize(rj_rdp* rdp, uint32_t width, uint32_t height);

int rj_clip_send_format_list(rj_rdp* rdp, uint32_t n, const uint32_t* ids, const char* const* names);
int rj_clip_request_data(rj_rdp* rdp, uint32_t format_id);
int rj_clip_send_data_response(rj_rdp* rdp, int ok, const uint8_t* data, uint32_t len);
int rj_clip_request_file(rj_rdp* rdp, uint32_t stream_id, uint32_t list_index, uint32_t flags,
                         uint64_t position, uint32_t requested);
int rj_clip_send_file_response(rj_rdp* rdp, uint32_t stream_id, int ok, const uint8_t* data,
                               uint32_t len);

#ifdef __cplusplus
}
#endif
#endif
