#ifndef RDP_BRIDGE_H
#define RDP_BRIDGE_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Minimal C bridge between DeskFuse (Swift) and FreeRDP's headless client
 * library. A single bridge instance owns one RDP connection; it runs FreeRDP's
 * event loop on a background thread and exposes a framebuffer plus input
 * injection. The caller owns credentials and UI; this bridge never persists
 * credentials or logs user content. */

typedef struct rdp_bridge rdp_bridge;

enum
{
	RDP_EVENT_CONNECTED = 1,
	RDP_EVENT_DISCONNECTED = 2,
	RDP_EVENT_AUTH_REQUIRED = 3, /* NLA or gateway asked for credentials */
	RDP_EVENT_ERROR = 4,
	RDP_EVENT_CERTIFICATE_REJECTED = 5, /* certificate was not explicitly trusted */
	RDP_EVENT_AUTH_FAILED = 6           /* credentials were rejected; prompt and retry */
};

/* `message` is a short, static UTF-8 description (never a password or body). */
typedef void (*rdp_bridge_callback)(int event, const char* message, const char* fingerprint,
                                    void* userdata);

/* `rdp_file` is a complete `.rdp` document (UTF-8, `full address` already
 * resolved). `username`/`password` may be NULL; credentials are supplied by
 * the Authenticate callback when the server requests them. An optional
 * certificate fingerprint enables explicit pinning; unknown or changed
 * certificates are rejected and reported through the callback. */
rdp_bridge* rdp_bridge_create(const char* rdp_file, const char* username,
                              const char* password, const char* certificate_fingerprint,
                              rdp_bridge_callback cb, void* userdata);

/* 0 on success; spawns the connect/event-loop thread. */
int rdp_bridge_start(rdp_bridge* bridge);

/* Transfers the socketpair's pump end to the caller exactly once. The caller
 * must close it or transfer ownership to Rust before starting FreeRDP. */
int rdp_bridge_take_tunnel_fd(rdp_bridge* bridge);

/* Queues an additional registered alias on the existing RAIL session. */
int rdp_bridge_execute(rdp_bridge* bridge, const char* name);

/* Requests disconnect and joins the background thread. Safe to call repeatedly. */
void rdp_bridge_stop(rdp_bridge* bridge);

/* 1 while the RDP session is fully connected. */
int rdp_bridge_connected(rdp_bridge* bridge);

/* Current framebuffer dimensions (0 when no frame is ready). */
int rdp_bridge_frame_size(rdp_bridge* bridge, uint32_t* width, uint32_t* height);

/* Copies the latest composed frame into `dst` (XRGB32 / BGRA byte order).
 * `dst_stride` is in bytes. Returns 0 on success, non-zero when no frame is
 * available or the destination is too small. */
int rdp_bridge_copy_frame(rdp_bridge* bridge, void* dst, uint32_t dst_stride, size_t capacity,
                          uint32_t expected_width, uint32_t expected_height);

/* Clipboard events. The bridge owns the cliprdr channel protocol; the caller
 * owns the local pasteboard and the byte conversions.
 *
 * RDP_CLIPBOARD_READY:   channel handshake finished; local formats can be published.
 * RDP_CLIPBOARD_FORMATS: `data` holds `length / 4` little-endian uint32 format ids
 *                        offered by the Windows side.
 * RDP_CLIPBOARD_DATA:    `format` holds the id requested earlier; `data` is the
 *                        payload, or NULL when Windows refused the request.
 * RDP_CLIPBOARD_REQUEST: Windows asked for local data in `format`; answer with
 *                        rdp_bridge_clipboard_set_data() for that format.
 */
enum
{
	RDP_CLIPBOARD_READY = 1,
	RDP_CLIPBOARD_FORMATS = 2,
	RDP_CLIPBOARD_DATA = 3,
	RDP_CLIPBOARD_REQUEST = 4
};

/* Runs on the FreeRDP event-loop thread; `data` is only valid for the call. */
typedef void (*rdp_clipboard_callback)(int event, uint32_t format, const uint8_t* data,
                                       size_t length, void* userdata);

void rdp_bridge_set_clipboard_callback(rdp_bridge* bridge, rdp_clipboard_callback cb,
                                       void* userdata);

/* Replaces the formats advertised to Windows. Formats not listed are dropped,
 * together with any data previously supplied for them. */
int rdp_bridge_clipboard_set_formats(rdp_bridge* bridge, const uint32_t* formats, size_t count);

/* Supplies the payload for one advertised format. When Windows already asked
 * for that format, the response is queued immediately. */
int rdp_bridge_clipboard_set_data(rdp_bridge* bridge, uint32_t format, const void* data,
                                  size_t length);

/* Requests the given format from the Windows clipboard. */
int rdp_bridge_clipboard_request(rdp_bridge* bridge, uint32_t format);

/* Enables or disables local audio playback. Call after create, before start:
 * FreeRDP loads the rdpsnd channel from this setting. */
void rdp_bridge_set_audio(rdp_bridge* bridge, int enabled);

/* Input injection; flags use FreeRDP's PTR_FLAGS_* / KBD_FLAGS_* values. */
/* Return 0 when the event was submitted, non-zero when the session is not
 * connected or the event is outside the current framebuffer/input bounds. */
int rdp_bridge_mouse(rdp_bridge* bridge, uint16_t x, uint16_t y, uint16_t flags);
int rdp_bridge_key(rdp_bridge* bridge, uint16_t flags, uint8_t scancode);
int rdp_bridge_unicode(rdp_bridge* bridge, uint16_t flags, uint16_t code);

void rdp_bridge_free(rdp_bridge* bridge);

#ifdef __cplusplus
}
#endif

#endif /* RDP_BRIDGE_H */
