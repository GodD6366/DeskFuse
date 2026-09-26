/* RdpBridge: headless FreeRDP client for uniDesk's Windows RemoteApp feature.
 *
 * One instance per RDP session. FreeRDP's connect and event loop run on a
 * dedicated WinPR thread; the composed GDI framebuffer is copied under a mutex
 * after each frame so the caller can read it from any thread without racing the
 * library's internal reallocations. */
#include "RdpBridge.h"

#include <freerdp/freerdp.h>
#include <freerdp/client.h>
#include <freerdp/client/cmdline.h>
#include <freerdp/client/channels.h>
#include <freerdp/channels/channels.h>
#include <freerdp/gdi/gdi.h>

#include <winpr/crt.h>
#include <winpr/assert.h>
#include <winpr/thread.h>
#include <winpr/synch.h>

#include <pthread.h>
#include <stdlib.h>
#include <string.h>
#include <stdatomic.h>

#define TAG "rdp.bridge"

typedef struct
{
	rdpClientContext common;
	struct rdp_bridge* bridge;
} bridgeContext;

struct rdp_bridge
{
	RDP_CLIENT_ENTRY_POINTS entry;
	rdpContext* context;
	freerdp* instance;
	HANDLE thread;

	char* rdp_file;
	char* username;
	char* password;

	rdp_bridge_callback cb;
	void* userdata;

	atomic_int connected;
	atomic_int stop_requested;

	pthread_mutex_t lock;
	uint8_t* pixels; /* bridge-owned copy of the composed frame */
	uint32_t width;
	uint32_t height;
	uint32_t stride; /* bytes per row of `pixels` */
	int frame_dirty;
};

static void notify(rdp_bridge* bridge, int event, const char* message)
{
	if (bridge->cb)
		bridge->cb(event, message, bridge->userdata);
}

/* --- certificate / authentication ------------------------------------- */

static DWORD bridge_verify_certificate_ex(freerdp* instance, const char* host, UINT16 port,
                                          const char* common_name, const char* subject,
                                          const char* issuer, const char* fingerprint, DWORD flags)
{
	/* Accept for the LAN-paired-device milestone. The uniDesk pairing proof is
	 * the trust boundary; a persistent-certificate check is a follow-up. */
	WINPR_UNUSED(instance);
	WINPR_UNUSED(host);
	WINPR_UNUSED(port);
	WINPR_UNUSED(common_name);
	WINPR_UNUSED(subject);
	WINPR_UNUSED(issuer);
	WINPR_UNUSED(fingerprint);
	WINPR_UNUSED(flags);
	return 1; /* accept */
}

static BOOL bridge_authenticate(freerdp* instance, char** username, char** password, char** domain)
{
	rdp_bridge* bridge = NULL;

	WINPR_UNUSED(domain);

	if (!instance || !instance->context)
		return FALSE;
	bridge = ((bridgeContext*)instance->context)->bridge;
	if (!bridge)
		return FALSE;

	notify(bridge, RDP_EVENT_AUTH_REQUIRED, "RDP 服务请求登录凭据");
	if (!bridge->username || !bridge->password)
		return FALSE;

	*username = _strdup(bridge->username);
	*password = _strdup(bridge->password);
	if (!*username || !*password)
	{
		free(*username);
		free(*password);
		*username = NULL;
		*password = NULL;
		return FALSE;
	}
	return TRUE;
}

/* --- update callbacks -------------------------------------------------- */

static BOOL bridge_begin_paint(rdpContext* context)
{
	rdpGdi* gdi = context->gdi;
	if (!gdi || !gdi->primary || !gdi->primary->hdc || !gdi->primary->hdc->hwnd)
		return FALSE;
	gdi->primary->hdc->hwnd->invalid->null = TRUE;
	return TRUE;
}

static BOOL bridge_end_paint(rdpContext* context)
{
	rdp_bridge* bridge = ((bridgeContext*)context)->bridge;
	rdpGdi* gdi = context->gdi;

	if (!bridge || !gdi || !gdi->primary_buffer || gdi->width <= 0 ||
        gdi->height <= 0 || gdi->width > 8192 || gdi->height > 8192 ||
        (size_t)gdi->stride * gdi->height > 256 * 1024 * 1024)
		return FALSE;

	pthread_mutex_lock(&bridge->lock);
	if (bridge->pixels == NULL || bridge->width != (uint32_t)gdi->width ||
	    bridge->height != (uint32_t)gdi->height || bridge->stride != gdi->stride)
	{
		size_t bytes = (size_t)gdi->stride * (size_t)gdi->height;
		uint8_t* next = (uint8_t*)realloc(bridge->pixels, bytes ? bytes : 1);
		if (!next)
		{
			pthread_mutex_unlock(&bridge->lock);
			return FALSE;
		}
		bridge->pixels = next;
		bridge->width = (uint32_t)gdi->width;
		bridge->height = (uint32_t)gdi->height;
		bridge->stride = gdi->stride;
	}

	if (gdi->width > 0 && gdi->height > 0 && bridge->pixels)
	{
		/* The primary buffer is one contiguous stride-per-row surface. */
		memcpy(bridge->pixels, gdi->primary_buffer,
		       (size_t)gdi->stride * (size_t)gdi->height);
		bridge->frame_dirty = 1;
	}
	pthread_mutex_unlock(&bridge->lock);
	return TRUE;
}

static BOOL bridge_desktop_resize(rdpContext* context)
{
	rdpGdi* gdi = context->gdi;
	rdpSettings* settings = context->settings;

	if (!gdi || !settings)
		return FALSE;
	return gdi_resize(gdi, freerdp_settings_get_uint32(settings, FreeRDP_DesktopWidth),
	                  freerdp_settings_get_uint32(settings, FreeRDP_DesktopHeight));
}

/* --- lifecycle --------------------------------------------------------- */

static BOOL bridge_pre_connect(freerdp* instance)
{
	rdpSettings* settings = instance->context->settings;

	if (!freerdp_settings_set_bool(settings, FreeRDP_CertificateCallbackPreferPEM, TRUE))
		return FALSE;
	return TRUE;
}

static BOOL bridge_post_connect(freerdp* instance)
{
	rdpContext* context = instance->context;

	if (!gdi_init(instance, PIXEL_FORMAT_XRGB32))
		return FALSE;

	if (!freerdp_settings_set_bool(context->settings, FreeRDP_DeactivateClientDecoding, TRUE))
		return FALSE;

	context->update->BeginPaint = bridge_begin_paint;
	context->update->EndPaint = bridge_end_paint;
	context->update->DesktopResize = bridge_desktop_resize;
	return TRUE;
}

static void bridge_post_disconnect(freerdp* instance)
{
	if (!instance || !instance->context)
		return;
	gdi_free(instance);
}

static BOOL bridge_client_new(freerdp* instance, rdpContext* context)
{
	WINPR_UNUSED(context);

	if (!instance)
		return FALSE;
	instance->PreConnect = bridge_pre_connect;
	instance->PostConnect = bridge_post_connect;
	instance->PostDisconnect = bridge_post_disconnect;
	instance->VerifyCertificateEx = bridge_verify_certificate_ex;
	instance->Authenticate = bridge_authenticate;
	return TRUE;
}

static void bridge_client_free(freerdp* instance, rdpContext* context)
{
	WINPR_UNUSED(instance);
	WINPR_UNUSED(context);
}

static int bridge_client_start(rdpContext* context)
{
	WINPR_UNUSED(context);
	return 0;
}

static int bridge_client_stop(rdpContext* context)
{
	WINPR_UNUSED(context);
	return 0;
}

static int bridge_entry(RDP_CLIENT_ENTRY_POINTS* entry)
{
	WINPR_ASSERT(entry);
	ZeroMemory(entry, sizeof(RDP_CLIENT_ENTRY_POINTS));
	entry->Version = RDP_CLIENT_INTERFACE_VERSION;
	entry->Size = sizeof(RDP_CLIENT_ENTRY_POINTS_V1);
	entry->ContextSize = sizeof(bridgeContext);
	entry->ClientNew = bridge_client_new;
	entry->ClientFree = bridge_client_free;
	entry->ClientStart = bridge_client_start;
	entry->ClientStop = bridge_client_stop;
	return 0;
}

/* --- connection thread ------------------------------------------------- */

static DWORD bridge_thread(LPVOID arg)
{
	rdp_bridge* bridge = (rdp_bridge*)arg;
	freerdp* instance = bridge->instance;
	DWORD status = 0;

	if (!freerdp_connect(instance))
	{
		status = freerdp_get_last_error(instance->context);
		notify(bridge, RDP_EVENT_ERROR, "RDP 连接失败");
		return status;
	}

	bridge->connected = 1;
	notify(bridge, RDP_EVENT_CONNECTED, NULL);

	while (!atomic_load(&bridge->stop_requested) && !freerdp_shall_disconnect_context(instance->context))
	{
		HANDLE handles[MAXIMUM_WAIT_OBJECTS] = { 0 };
		DWORD count = freerdp_get_event_handles(instance->context, handles, MAXIMUM_WAIT_OBJECTS);

		if (count == 0)
			break;

		status = WaitForMultipleObjects(count, handles, FALSE, 100);
		if (status == WAIT_FAILED)
			break;

		if (!freerdp_check_event_handles(instance->context))
		{
			if (freerdp_get_last_error(instance->context) == FREERDP_ERROR_SUCCESS)
				break;
		}
	}

	freerdp_disconnect(instance);
	bridge->connected = 0;
	notify(bridge, RDP_EVENT_DISCONNECTED, NULL);
	return 0;
}

/* --- public API -------------------------------------------------------- */

rdp_bridge* rdp_bridge_create(const char* rdp_file, const char* username, const char* password,
                              rdp_bridge_callback cb, void* userdata)
{
	rdp_bridge* bridge = NULL;

	if (!rdp_file)
		return NULL;

	bridge = (rdp_bridge*)calloc(1, sizeof(rdp_bridge));
	if (!bridge)
		return NULL;

	pthread_mutex_init(&bridge->lock, NULL);
	bridge->rdp_file = _strdup(rdp_file);
	bridge->username = username ? _strdup(username) : NULL;
	bridge->password = password ? _strdup(password) : NULL;
	bridge->cb = cb;
	bridge->userdata = userdata;

	bridge_entry(&bridge->entry);
	bridge->context = freerdp_client_context_new(&bridge->entry);
	if (!bridge->context)
		goto fail;

	((bridgeContext*)bridge->context)->bridge = bridge;

	if (freerdp_client_settings_parse_connection_file_buffer(
	        bridge->context->settings, (const BYTE*)bridge->rdp_file, strlen(bridge->rdp_file)) != 0)
		goto fail;

	if (bridge->username)
		freerdp_settings_set_string(bridge->context->settings, FreeRDP_Username, bridge->username);
	if (bridge->password)
		freerdp_settings_set_string(bridge->context->settings, FreeRDP_Password, bridge->password);
	freerdp_settings_set_bool(bridge->context->settings, FreeRDP_AutoAcceptCertificate, TRUE);
	freerdp_settings_set_bool(bridge->context->settings, FreeRDP_CertificateCallbackPreferPEM, TRUE);

	bridge->instance = bridge->context->instance;
	return bridge;

fail:
	rdp_bridge_free(bridge);
	return NULL;
}

int rdp_bridge_start(rdp_bridge* bridge)
{
	if (!bridge || !bridge->context)
		return -1;
	if (freerdp_client_start(bridge->context) != 0)
		return -1;
	bridge->thread = CreateThread(NULL, 0, bridge_thread, bridge, 0, NULL);
	if (!bridge->thread)
		return -1;
	return 0;
}

void rdp_bridge_stop(rdp_bridge* bridge)
{
	if (!bridge)
		return;
	bridge->stop_requested = 1;
	if (bridge->thread && bridge->context && !bridge->connected)
		freerdp_abort_connect_context(bridge->context);
	if (bridge->thread)
	{
		/* Never free context/userdata while the callback thread still runs. */
        WaitForSingleObject(bridge->thread, INFINITE);
		CloseHandle(bridge->thread);
		bridge->thread = NULL;
	}
}

int rdp_bridge_connected(rdp_bridge* bridge)
{
	return bridge ? bridge->connected : 0;
}

int rdp_bridge_frame_size(rdp_bridge* bridge, uint32_t* width, uint32_t* height)
{
	if (!bridge)
		return -1;
	pthread_mutex_lock(&bridge->lock);
	if (bridge->frame_dirty && bridge->pixels)
	{
		*width = bridge->width;
		*height = bridge->height;
		pthread_mutex_unlock(&bridge->lock);
		return 0;
	}
	pthread_mutex_unlock(&bridge->lock);
	return -1;
}

int rdp_bridge_copy_frame(rdp_bridge* bridge, void* dst, uint32_t dst_stride, size_t capacity,
                          uint32_t expected_width, uint32_t expected_height)
{
	uint32_t rows = 0;
	uint32_t row_bytes = 0;

	if (!bridge || !dst)
		return -1;

	pthread_mutex_lock(&bridge->lock);
	if (!bridge->frame_dirty || !bridge->pixels || bridge->width == 0 || bridge->height == 0)
	{
		pthread_mutex_unlock(&bridge->lock);
		return -1;
	}
	row_bytes = bridge->width * 4;
	if (bridge->width != expected_width || bridge->height != expected_height ||
        dst_stride < row_bytes || capacity / dst_stride < bridge->height)
	{
		pthread_mutex_unlock(&bridge->lock);
		return -1;
	}
	for (rows = 0; rows < bridge->height; rows++)
	{
		memcpy((uint8_t*)dst + (size_t)rows * dst_stride,
		       bridge->pixels + (size_t)rows * bridge->stride, row_bytes);
	}
	bridge->frame_dirty = 0;
	pthread_mutex_unlock(&bridge->lock);
	return 0;
}

void rdp_bridge_mouse(rdp_bridge* bridge, uint16_t x, uint16_t y, uint16_t flags)
{
	if (bridge && bridge->context && bridge->context->input)
		freerdp_input_send_mouse_event(bridge->context->input, flags, x, y);
}

void rdp_bridge_key(rdp_bridge* bridge, uint16_t flags, uint8_t scancode)
{
	if (bridge && bridge->context && bridge->context->input)
		freerdp_input_send_keyboard_event(bridge->context->input, flags, scancode);
}

void rdp_bridge_unicode(rdp_bridge* bridge, uint16_t flags, uint16_t code)
{
	if (bridge && bridge->context && bridge->context->input)
		freerdp_input_send_unicode_keyboard_event(bridge->context->input, flags, code);
}

void rdp_bridge_free(rdp_bridge* bridge)
{
	if (!bridge)
		return;

	rdp_bridge_stop(bridge);

	if (bridge->context)
	{
		freerdp_client_stop(bridge->context);
		freerdp_client_context_free(bridge->context);
		bridge->context = NULL;
	}

	free(bridge->rdp_file);
	free(bridge->username);
	free(bridge->password);
	free(bridge->pixels);
	pthread_mutex_destroy(&bridge->lock);
	free(bridge);
}
