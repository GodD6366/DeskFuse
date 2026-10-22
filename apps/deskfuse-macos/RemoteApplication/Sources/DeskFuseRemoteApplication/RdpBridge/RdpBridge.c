/* RdpBridge: headless FreeRDP client for DeskFuse's Windows RemoteApp feature.
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
#include <freerdp/client/rail.h>
#include <freerdp/channels/channels.h>
#include <freerdp/crypto/certificate.h>
#include <freerdp/gdi/gdi.h>
#include <freerdp/transport_io.h>
#include <sys/socket.h>
#include <unistd.h>
#include <fcntl.h>
#include <poll.h>
#include <errno.h>

#include <winpr/crt.h>
#include <winpr/assert.h>
#include <winpr/thread.h>
#include <winpr/synch.h>
#include <winpr/winsock.h>

#include <pthread.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <ctype.h>
#include <stdatomic.h>
#include <limits.h>

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
	int tunnel_fd;
	int transport_fd;
	RailClientContext* rail;
	int rail_ready;
	char pending_program[16][132];
    unsigned pending_count;
	pthread_mutex_t command_lock;

	char* rdp_file;
	char* username;
	char* password;
	char* certificate_fingerprint;

	rdp_bridge_callback cb;
	void* userdata;

	atomic_int connected;
	atomic_int stop_requested;
	atomic_int certificate_rejected;
	/* Windows 下发 LOGON_MSG_BUMP_OPTIONS：已有其他会话，需挤占才能登录。 */
	atomic_int logon_bump_requested;

	pthread_mutex_t lock;
	uint8_t* pixels; /* bridge-owned copy of the composed frame */
	uint32_t width;
	uint32_t height;
	uint32_t stride; /* bytes per row of `pixels` */
	int frame_dirty;
};

static void notify_detail(rdp_bridge* bridge, int event, const char* message,
                          const char* fingerprint)
{
	if (bridge->cb)
		bridge->cb(event, message, fingerprint, bridge->userdata);
}

static void notify(rdp_bridge* bridge, int event, const char* message)
{
	notify_detail(bridge, event, message, NULL);
}

static int valid_fingerprint(const char* fingerprint)
{
	if (!fingerprint || strlen(fingerprint) != 95)
		return 0;
	for (size_t index = 0; index < 95; index++)
	{
		if ((index % 3) == 2)
		{
			if (fingerprint[index] != ':')
				return 0;
		}
		else if (!isxdigit((unsigned char)fingerprint[index]))
			return 0;
	}
	return 1;
}

static int fingerprints_match(const char* expected, const char* actual)
{
	return valid_fingerprint(expected) && valid_fingerprint(actual) && _stricmp(expected, actual) == 0;
}

/* FreeRDP 3.14's default ConnectLayer bypasses TCPConnect. Override both:
 * all I/O must use this socketpair, even if a proxy is set in the environment. */
typedef struct { int fd; HANDLE event; } bridgeSocket;

static int bridge_tcp_connect(rdpContext* context, rdpSettings* settings,
                              const char* host, int port, DWORD timeout)
{
    WINPR_UNUSED(settings); WINPR_UNUSED(host); WINPR_UNUSED(port); WINPR_UNUSED(timeout);
    rdp_bridge* bridge = ((bridgeContext*)context)->bridge;
    int fd = bridge->transport_fd;
    bridge->transport_fd = -1;
    return fd;
}
static int bridge_socket_read(void* data, void* buffer, int size)
{
    bridgeSocket* socket = data;
    WSAResetEvent(socket->event);
    int n = (int)recv(socket->fd, buffer, (size_t)size, 0);
    if (n == 0) return -1;
    if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK || errno == EINTR)) return 0;
    return n;
}
static int bridge_socket_write(void* data, const void* buffer, int size)
{
    bridgeSocket* socket = data;
    int n = (int)send(socket->fd, buffer, (size_t)size, 0);
    if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK || errno == EINTR)) return 0;
    return n;
}
static BOOL bridge_socket_close(void* data)
{
    bridgeSocket* socket = data;
    if (socket->fd >= 0) { shutdown(socket->fd, SHUT_RDWR); close(socket->fd); socket->fd = -1; }
    if (socket->event) { WSACloseEvent(socket->event); socket->event = NULL; }
    return TRUE;
}
static BOOL bridge_socket_wait(void* data, BOOL write, DWORD timeout)
{
    bridgeSocket* socket = data;
    struct pollfd item = { .fd = socket->fd, .events = write ? POLLOUT : POLLIN };
    int result;
    do { result = poll(&item, 1, timeout == INFINITE ? -1 : (int)timeout); } while (result < 0 && errno == EINTR);
    return result > 0 && !(item.revents & (POLLERR | POLLNVAL));
}
static HANDLE bridge_socket_event(void* data) { return ((bridgeSocket*)data)->event; }
static rdpTransportLayer* bridge_connect_layer(rdpTransport* transport, const char* host,
                                               int port, DWORD timeout)
{
    rdpContext* context = transport_get_context(transport);
    rdpTransportLayer* layer = transport_layer_new(transport, sizeof(bridgeSocket));
    if (!layer) return NULL;
    bridgeSocket* socket = layer->userContext;
    socket->fd = -1;
    layer->Close = bridge_socket_close;
    socket->fd = freerdp_get_io_callbacks(context)->TCPConnect(context, context->settings, host, port, timeout);
    if (socket->fd < 0) goto fail;
    socket->event = WSACreateEvent();
    if (!socket->event || WSAEventSelect(socket->fd, socket->event, FD_READ | FD_CLOSE)) goto fail;
    layer->Read = bridge_socket_read;
    layer->Write = bridge_socket_write;
    layer->Wait = bridge_socket_wait;
    layer->GetEvent = bridge_socket_event;
    return layer;
fail:
    bridge_socket_close(socket);
    transport_layer_free(layer);
    return NULL;
}

/* --- certificate / authentication ------------------------------------- */

static int bridge_verify_x509_certificate(freerdp* instance, const BYTE* data, size_t length,
                                          const char* host, UINT16 port, DWORD flags)
{
	char* pem = NULL;
	rdpCertificate* certificate = NULL;
	char* fingerprint = NULL;
	int result = 0;
	WINPR_UNUSED(port);
	WINPR_UNUSED(flags);
	if (!instance || !instance->context || !data || length == 0 || length > 1024 * 1024)
		return 0;
	rdp_bridge* bridge = ((bridgeContext*)instance->context)->bridge;
	if (!bridge)
		return 0;
	pem = (char*)calloc(length + 1, 1);
	if (!pem)
		return 0;
	memcpy(pem, data, length);
	certificate = freerdp_certificate_new_from_pem(pem);
	if (certificate)
		fingerprint = freerdp_certificate_get_fingerprint(certificate);
	if (fingerprints_match(bridge->certificate_fingerprint, fingerprint))
	{
		atomic_store(&bridge->certificate_rejected, 0);
		result = 2; /* accept for this session; the app owns the persistent pin */
	}
	else
	{
		atomic_store(&bridge->certificate_rejected, 1);
		notify_detail(bridge, RDP_EVENT_CERTIFICATE_REJECTED,
		              "RDP 证书与已配对 Windows 下发的指纹不一致，请刷新目录后重试", fingerprint);
	}
	free(fingerprint);
	freerdp_certificate_free(certificate);
	free(pem);
	return result;
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

/* RemoteApp sessions are bound to the paired host and its pinned certificate.
 * FreeRDP can otherwise accept a redirection certificate through
 * RedirectionAcceptedCert before VerifyX509Certificate runs.  A redirected
 * farm target would therefore escape the application's single-host pin, so
 * fail closed until explicit multi-host pinning is designed. */
static BOOL bridge_redirect(freerdp* instance)
{
	if (instance && instance->context)
	{
		rdp_bridge* bridge = ((bridgeContext*)instance->context)->bridge;
		if (bridge)
			notify(bridge, RDP_EVENT_ERROR, "RDP 服务器请求重定向，DeskFuse 已拒绝");
	}
	return FALSE;
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
	    gdi->stride <= 0 || (size_t)gdi->stride < (size_t)gdi->width * 4 ||
	    (size_t)gdi->stride > SIZE_MAX / (size_t)gdi->height ||
	    (size_t)gdi->stride * (size_t)gdi->height > 256 * 1024 * 1024)
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

/* RemoteApp does not launch merely by connecting: the RAIL handshake must
 * send client status, work area and the requested executable. */
static UINT bridge_rail_handshake(RailClientContext* context, const RAIL_HANDSHAKE_ORDER* order)
{
	WINPR_UNUSED(order);
	rdp_bridge* bridge = context->custom;
    if (bridge->rail_ready) return CHANNEL_RC_OK;
    UINT result = client_rail_server_start_cmd(context);
    if (result == CHANNEL_RC_OK) bridge->rail_ready = 1;
    return result;
}

static UINT bridge_rail_handshake_ex(RailClientContext* context, const RAIL_HANDSHAKE_EX_ORDER* order)
{
	WINPR_UNUSED(order);
	rdp_bridge* bridge = context->custom;
    if (bridge->rail_ready) return CHANNEL_RC_OK;
    UINT result = client_rail_server_start_cmd(context);
    if (result == CHANNEL_RC_OK) bridge->rail_ready = 1;
    return result;
}

static UINT bridge_rail_execute_result(RailClientContext* context, const RAIL_EXEC_RESULT_ORDER* result)
{
	rdp_bridge* bridge = context->custom;
	if (result->execResult != RAIL_EXEC_S_OK)
	{
		notify(bridge, RDP_EVENT_ERROR, "Windows 无法启动所选远程应用");
		atomic_store(&bridge->stop_requested, 1);
	}
	return CHANNEL_RC_OK;
}

static void bridge_channel_connected(void* context, const ChannelConnectedEventArgs* event)
{
	bridgeContext* client = context;
	if (strcmp(event->name, RAIL_SVC_CHANNEL_NAME) == 0)
	{
		RailClientContext* rail = event->pInterface;
		rail->custom = client->bridge;
        client->bridge->rail = rail;
        client->bridge->rail_ready = 0;
		rail->ServerHandshake = bridge_rail_handshake;
		rail->ServerHandshakeEx = bridge_rail_handshake_ex;
		rail->ServerExecuteResult = bridge_rail_execute_result;
	}
	else
		freerdp_client_OnChannelConnectedEventHandler(context, event);
}

static void bridge_channel_disconnected(void* context, const ChannelDisconnectedEventArgs* event)
{
	if (strcmp(event->name, RAIL_SVC_CHANNEL_NAME) == 0)
	{
		RailClientContext* rail = event->pInterface;
		((bridgeContext*)context)->bridge->rail = NULL;
        ((bridgeContext*)context)->bridge->rail_ready = 0;
        rail->custom = NULL;
		rail->ServerHandshake = NULL;
		rail->ServerHandshakeEx = NULL;
		rail->ServerExecuteResult = NULL;
	}
	else
		freerdp_client_OnChannelDisconnectedEventHandler(context, event);
}

static BOOL bridge_pre_connect(freerdp* instance)
{
	rdpSettings* settings = instance->context->settings;

	if (!freerdp_settings_set_bool(settings, FreeRDP_ExternalCertificateManagement, TRUE) ||
	    !freerdp_settings_set_bool(settings, FreeRDP_IgnoreCertificate, FALSE) ||
	    !freerdp_settings_set_bool(settings, FreeRDP_AutoAcceptCertificate, FALSE) ||
	    !freerdp_settings_set_string(settings, FreeRDP_CertificateAcceptedFingerprints, NULL))
		return FALSE;
	PubSub_SubscribeChannelConnected(instance->context->pubSub, bridge_channel_connected);
	PubSub_SubscribeChannelDisconnected(instance->context->pubSub, bridge_channel_disconnected);
	return TRUE;
}

static BOOL bridge_post_connect(freerdp* instance)
{
	rdpContext* context = instance->context;

	if (!gdi_init(instance, PIXEL_FORMAT_XRGB32))
		return FALSE;

	/* Keep GDI bitmap/glyph decoding enabled; this client displays decoded pixels. */

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

/* RemoteApp 模式没有可见的登录界面，记录登录提示以便会话结束时给出原因。 */
static int bridge_logon_error_info(freerdp* instance, UINT32 data, UINT32 type)
{
	WINPR_UNUSED(data);
	if (!instance || !instance->context)
		return 1;
	rdp_bridge* bridge = ((bridgeContext*)instance->context)->bridge;
	/* errorNotificationType 携带 LOGON_MSG_*，data 是对应的失败码或会话号。 */
	if (bridge && type == LOGON_MSG_BUMP_OPTIONS)
		atomic_store(&bridge->logon_bump_requested, 1);
	return 1;
}

static BOOL bridge_client_new(freerdp* instance, rdpContext* context)
{
	WINPR_UNUSED(context);

	if (!instance)
		return FALSE;
	instance->PreConnect = bridge_pre_connect;
	instance->PostConnect = bridge_post_connect;
	instance->PostDisconnect = bridge_post_disconnect;
	instance->VerifyX509Certificate = bridge_verify_x509_certificate;
	instance->Authenticate = bridge_authenticate;
	instance->Redirect = bridge_redirect;
	instance->LogonErrorInfo = bridge_logon_error_info;
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

/* Launch commands are consumed only on FreeRDP's event-loop thread. */
static void bridge_drain_execute(rdp_bridge* bridge)
{
    if (!bridge->rail || !bridge->rail_ready) return;
    char program[132] = {0};
    pthread_mutex_lock(&bridge->command_lock);
    if (bridge->pending_count) {
        memcpy(program, bridge->pending_program[0], sizeof(program));
        bridge->pending_count--;
        memmove(bridge->pending_program, bridge->pending_program + 1,
                bridge->pending_count * sizeof(bridge->pending_program[0]));
    }
    pthread_mutex_unlock(&bridge->command_lock);
    if (!program[0]) return;
    RAIL_EXEC_ORDER order = {0};
    order.flags = RAIL_EXEC_FLAG_EXPAND_ARGUMENTS;
    order.RemoteApplicationProgram = program;
    order.RemoteApplicationWorkingDir = "";
    order.RemoteApplicationArguments = "";
    if (bridge->rail->ClientExecute(bridge->rail, &order) != CHANNEL_RC_OK)
        notify(bridge, RDP_EVENT_ERROR, "Windows 远程应用启动请求发送失败，请重试");
}

int rdp_bridge_execute(rdp_bridge* bridge, const char* name)
{
    if (!bridge || !name || !*name || strlen(name) > 128) return -1;
    for (const unsigned char* p = (const unsigned char*)name; *p; p++)
        if (*p < 32 || *p == 127 || *p == '/' || *p == '\\' || *p == ':' || *p == '"') return -1;
    pthread_mutex_lock(&bridge->command_lock);
    int result = -1;
    if (bridge->pending_count < 16) {
        snprintf(bridge->pending_program[bridge->pending_count++], 132, "||%s", name);
        result = 0;
    }
    pthread_mutex_unlock(&bridge->command_lock);
    return result;
}

/* 把 FreeRDP 错误码翻译成可操作的中文原因，并附上错误名便于从日志定位。 */
static const char* connect_failure_reason(UINT32 code)
{
	switch (code)
	{
		case FREERDP_ERROR_AUTHENTICATION_FAILED:
		case FREERDP_ERROR_CONNECT_LOGON_FAILURE:
		case FREERDP_ERROR_CONNECT_WRONG_PASSWORD:
		case FREERDP_ERROR_CONNECT_NO_OR_MISSING_CREDENTIALS:
			return "Windows 登录失败：用户名或密码不正确。Microsoft 账户请填写完整邮箱和账户密码（不是 PIN）";
		case FREERDP_ERROR_CONNECT_PASSWORD_EXPIRED:
		case FREERDP_ERROR_CONNECT_PASSWORD_CERTAINLY_EXPIRED:
		case FREERDP_ERROR_CONNECT_PASSWORD_MUST_CHANGE:
			return "Windows 账户密码已过期或需要修改，请先在 Windows 上更新密码";
		case FREERDP_ERROR_CONNECT_ACCOUNT_DISABLED:
		case FREERDP_ERROR_CONNECT_ACCOUNT_LOCKED_OUT:
		case FREERDP_ERROR_CONNECT_ACCOUNT_EXPIRED:
			return "Windows 账户已被禁用、锁定或过期";
		case FREERDP_ERROR_INSUFFICIENT_PRIVILEGES:
		case FREERDP_ERROR_CONNECT_ACCESS_DENIED:
		case FREERDP_ERROR_CONNECT_ACCOUNT_RESTRICTION:
		case FREERDP_ERROR_CONNECT_LOGON_TYPE_NOT_GRANTED:
		case FREERDP_ERROR_SERVER_DENIED_CONNECTION:
			return "该账户没有远程桌面登录权限：请在 Windows 上将其加入“Remote Desktop Users”，或使用管理员账户";
		case FREERDP_ERROR_SECURITY_NEGO_CONNECT_FAILED:
		case FREERDP_ERROR_TLS_CONNECT_FAILED:
			return "与 Windows 远程桌面的安全协商失败，请确认 Windows 已开启远程桌面并允许网络级身份验证";
		case FREERDP_ERROR_CONNECT_TRANSPORT_FAILED:
		case FREERDP_ERROR_CONNECT_FAILED:
			return "无法通过 DeskFuse 隧道连接 Windows 远程桌面服务，请确认 Windows 已开启远程桌面";
		case FREERDP_ERROR_CONNECT_ACTIVATION_TIMEOUT:
			return "Windows 远程桌面响应超时，请稍后重试";
		default:
			return "RDP 连接失败";
	}
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
		/* The certificate callback already emitted the actionable message. */
		if (!atomic_load(&bridge->certificate_rejected))
		{
			/* 回调同步拷贝字符串，栈缓冲区在 notify 返回前有效。 */
			char message[512];
			const char* name = freerdp_get_last_error_name(status);
			snprintf(message, sizeof(message), "%s（%s）", connect_failure_reason(status),
			         name ? name : "UNKNOWN");
			notify(bridge, RDP_EVENT_ERROR, message);
		}
		return status;
	}

	bridge->connected = 1;
	notify(bridge, RDP_EVENT_CONNECTED, NULL);

	while (!atomic_load(&bridge->stop_requested) && !freerdp_shall_disconnect_context(instance->context))
	{
		bridge_drain_execute(bridge);
        HANDLE handles[MAXIMUM_WAIT_OBJECTS] = { 0 };
		DWORD count = freerdp_get_event_handles(instance->context, handles, MAXIMUM_WAIT_OBJECTS);

		if (count == 0)
			break;

		status = WaitForMultipleObjects(count, handles, FALSE, 100);
		if (status == WAIT_FAILED)
			break;

		if (!freerdp_check_event_handles(instance->context))
		{
			break;
		}
	}

	/* 服务端结束会话时把原因翻译给用户；本地主动停止不算错误。 */
	const UINT32 last_error = freerdp_get_last_error(instance->context);
	if (!atomic_load(&bridge->stop_requested))
	{
		if (atomic_load(&bridge->logon_bump_requested))
			notify(bridge, RDP_EVENT_ERROR,
			       "Windows 上已有另一个登录会话，远程应用登录被拒绝（Windows 专业版同一时间只允许一个会话）。"
			       "请先在 Windows 上注销或断开当前会话后重试");
		else if (last_error == FREERDP_ERROR_LOGOFF_BY_USER)
			notify(bridge, RDP_EVENT_ERROR, "Windows 已注销该远程会话，请重新打开应用");
		else if (last_error != FREERDP_ERROR_SUCCESS)
		{
			char message[256];
			const char* name = freerdp_get_last_error_name(last_error);
			snprintf(message, sizeof(message), "RDP 会话传输失败（%s）", name ? name : "UNKNOWN");
			notify(bridge, RDP_EVENT_ERROR, message);
		}
	}

	freerdp_disconnect(instance);
	bridge->connected = 0;
	notify(bridge, RDP_EVENT_DISCONNECTED, NULL);
	return 0;
}

/* --- public API -------------------------------------------------------- */

rdp_bridge* rdp_bridge_create(const char* rdp_file, const char* username, const char* password,
                              const char* certificate_fingerprint, rdp_bridge_callback cb,
                              void* userdata)
{
	rdp_bridge* bridge = NULL;

	if (!rdp_file)
		return NULL;

	bridge = (rdp_bridge*)calloc(1, sizeof(rdp_bridge));
	if (!bridge)
		return NULL;

	bridge->tunnel_fd = -1;
    bridge->transport_fd = -1;
	pthread_mutex_init(&bridge->lock, NULL);
    pthread_mutex_init(&bridge->command_lock, NULL);
    int pair[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, pair) != 0) goto fail;
    bridge->transport_fd = pair[0]; bridge->tunnel_fd = pair[1];
    for (int i = 0; i < 2; i++) {
        fcntl(pair[i], F_SETFD, FD_CLOEXEC);
        int enabled = 1; setsockopt(pair[i], SOL_SOCKET, SO_NOSIGPIPE, &enabled, sizeof(enabled));
    }
	bridge->rdp_file = _strdup(rdp_file);
	bridge->username = username ? _strdup(username) : NULL;
	bridge->password = password ? _strdup(password) : NULL;
	bridge->certificate_fingerprint = certificate_fingerprint ? _strdup(certificate_fingerprint) : NULL;
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
	bridge->instance = bridge->context->instance;
    rdpTransportIo io = *freerdp_get_io_callbacks(bridge->context);
    io.TCPConnect = bridge_tcp_connect;
    io.ConnectLayer = bridge_connect_layer;
    if (!freerdp_set_io_callbacks(bridge->context, &io)) goto fail;
    /* No gateway, proxy, UDP or redirection can escape the authenticated tunnel. */
    freerdp_settings_set_bool(bridge->context->settings, FreeRDP_GatewayEnabled, FALSE);
    freerdp_settings_set_uint32(bridge->context->settings, FreeRDP_ProxyType, 0);
    freerdp_settings_set_bool(bridge->context->settings, FreeRDP_AutoReconnectionEnabled, FALSE);
    /* The server must not initiate an out-of-band RDP-UDP transport: only the
     * socketpair-backed TCP stream is reachable from this process. */
    freerdp_settings_set_bool(bridge->context->settings, FreeRDP_SupportMultitransport, FALSE);
    freerdp_settings_set_uint32(bridge->context->settings, FreeRDP_MultitransportFlags, 0);
    /* FreeRDP 默认开启网络自动探测与心跳 PDU，并据此强制启用 rdpdr 设备重定向；
     * 本构建未编入 rdpdr 插件，加载失败会让每次连接都以 PRE_CONNECT_FAILED 结束。
     * 隧道内只有单条 TCP 流，两者都不需要，也不开放任何设备重定向。 */
    freerdp_settings_set_bool(bridge->context->settings, FreeRDP_NetworkAutoDetect, FALSE);
    freerdp_settings_set_bool(bridge->context->settings, FreeRDP_SupportHeartbeatPdu, FALSE);
    freerdp_settings_set_bool(bridge->context->settings, FreeRDP_DeviceRedirection, FALSE);
    /* Cliprdr is compiled in but only wired during P4; do not half-enable it. */
    freerdp_settings_set_bool(bridge->context->settings, FreeRDP_RedirectClipboard, FALSE);
	return bridge;

fail:
	rdp_bridge_free(bridge);
	return NULL;
}

int rdp_bridge_take_tunnel_fd(rdp_bridge* bridge)
{
    if (!bridge) return -1;
    int fd = bridge->tunnel_fd; bridge->tunnel_fd = -1; return fd;
}

int rdp_bridge_start(rdp_bridge* bridge)
{
	if (!bridge || !bridge->context || bridge->thread || !valid_fingerprint(bridge->certificate_fingerprint))
		return -1;
	atomic_store(&bridge->stop_requested, 0);
	atomic_store(&bridge->connected, 0);
	atomic_store(&bridge->certificate_rejected, 0);
	atomic_store(&bridge->logon_bump_requested, 0);
	if (freerdp_client_start(bridge->context) != 0)
		return -1;
	bridge->thread = CreateThread(NULL, 0, bridge_thread, bridge, 0, NULL);
	if (!bridge->thread)
	{
		freerdp_client_stop(bridge->context);
		atomic_store(&bridge->stop_requested, 0);
		return -1;
	}
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
	/* freerdp_client_start allocates client-side state which must be released
	 * even when the connection thread exits before freerdp_connect succeeds. */
	if (bridge->context)
		freerdp_client_stop(bridge->context);
	atomic_store(&bridge->connected, 0);
	atomic_store(&bridge->stop_requested, 0);
	pthread_mutex_lock(&bridge->lock);
	bridge->frame_dirty = 0;
	pthread_mutex_unlock(&bridge->lock);
}

int rdp_bridge_connected(rdp_bridge* bridge)
{
	return bridge ? bridge->connected : 0;
}

int rdp_bridge_frame_size(rdp_bridge* bridge, uint32_t* width, uint32_t* height)
{
	if (!bridge || !width || !height)
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
	if (bridge->width > UINT32_MAX / 4)
	{
		pthread_mutex_unlock(&bridge->lock);
		return -1;
	}
	row_bytes = bridge->width * 4;
	if (bridge->width != expected_width || bridge->height != expected_height ||
	    dst_stride < row_bytes || dst_stride == 0 ||
        (size_t)dst_stride > SIZE_MAX / bridge->height ||
        capacity < (size_t)dst_stride * bridge->height)
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

int rdp_bridge_mouse(rdp_bridge* bridge, uint16_t x, uint16_t y, uint16_t flags)
{
	if (!bridge || !atomic_load(&bridge->connected) || !bridge->context || !bridge->context->input)
		return -1;
	pthread_mutex_lock(&bridge->lock);
	if (bridge->width == 0 || bridge->height == 0 || x >= bridge->width || y >= bridge->height)
	{
		pthread_mutex_unlock(&bridge->lock);
		return -1;
	}
	pthread_mutex_unlock(&bridge->lock);
	return freerdp_input_send_mouse_event(bridge->context->input, flags, x, y) ? 0 : -1;
}

int rdp_bridge_key(rdp_bridge* bridge, uint16_t flags, uint8_t scancode)
{
	if (!bridge || !atomic_load(&bridge->connected) || !bridge->context || !bridge->context->input)
		return -1;
	return freerdp_input_send_keyboard_event(bridge->context->input, flags, scancode) ? 0 : -1;
}

int rdp_bridge_unicode(rdp_bridge* bridge, uint16_t flags, uint16_t code)
{
	if (!bridge || !atomic_load(&bridge->connected) || !bridge->context || !bridge->context->input || code == 0)
		return -1;
	return freerdp_input_send_unicode_keyboard_event(bridge->context->input, flags, code) ? 0 : -1;
}

void rdp_bridge_free(rdp_bridge* bridge)
{
	if (!bridge)
		return;

    rdp_bridge_stop(bridge);

    if (bridge->context)
    {
        const char* stored_password = freerdp_settings_get_string(bridge->context->settings, FreeRDP_Password);
        if (stored_password) SecureZeroMemory((void*)stored_password, strlen(stored_password));
        freerdp_client_context_free(bridge->context);
        bridge->context = NULL;
    }

	free(bridge->rdp_file);
	free(bridge->username);
	if (bridge->password) SecureZeroMemory(bridge->password, strlen(bridge->password));
	free(bridge->password);
	free(bridge->certificate_fingerprint);
	free(bridge->pixels);
	pthread_mutex_destroy(&bridge->lock);
    pthread_mutex_destroy(&bridge->command_lock);
    if (bridge->tunnel_fd >= 0) close(bridge->tunnel_fd);
    if (bridge->transport_fd >= 0) close(bridge->transport_fd);
	free(bridge);
}
