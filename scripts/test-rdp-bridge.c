/* White-box native regression tests against the pinned FreeRDP headers/library. */
#include <assert.h>
#include <openssl/evp.h>
#include <openssl/pem.h>
#include <openssl/rsa.h>
#include <openssl/x509.h>
#include "../apps/deskfuse-macos/RemoteApplication/Sources/DeskFuseRemoteApplication/RdpBridge/RdpBridge.c"
#include "rail_main.h"

static unsigned information_calls, system_calls, execute_calls, error_calls;
static unsigned certificate_rejections;
static unsigned clipboard_requests, clipboard_format_lists, clipboard_list_responses;
static uint32_t clipboard_requested_format;
static uint8_t clipboard_response[64];
static size_t clipboard_response_length;
static uint16_t clipboard_response_flags;
static char rejected_fingerprint[96];
static int rejected_fingerprint_was_null;

static void clipboard_event(int kind, uint32_t format, const uint8_t* data, size_t length,
                            void* userdata)
{
    (void)data;
    (void)length;
    (void)userdata;
    if (kind == RDP_CLIPBOARD_REQUEST)
    {
        clipboard_requests++;
        clipboard_requested_format = format;
    }
    if (kind == RDP_CLIPBOARD_FORMATS)
        clipboard_format_lists++;
}

static UINT clipboard_format_list_response(CliprdrClientContext* context,
                                           const CLIPRDR_FORMAT_LIST_RESPONSE* response)
{
    (void)context;
    assert(response->common.msgFlags == CB_RESPONSE_OK);
    clipboard_list_responses++;
    return CHANNEL_RC_OK;
}

static UINT clipboard_format_data_response(CliprdrClientContext* context,
                                           const CLIPRDR_FORMAT_DATA_RESPONSE* response)
{
    (void)context;
    clipboard_response_flags = response->common.msgFlags;
    clipboard_response_length = response->common.dataLen;
    assert(clipboard_response_length <= sizeof(clipboard_response));
    if (response->requestedFormatData && clipboard_response_length)
        memcpy(clipboard_response, response->requestedFormatData, clipboard_response_length);
    return CHANNEL_RC_OK;
}
static UINT information(RailClientContext* rail, const RAIL_CLIENT_STATUS_ORDER* order)
{
    assert(rail && order);
    information_calls++;
    return CHANNEL_RC_OK;
}
static UINT system_parameters(RailClientContext* rail, const RAIL_SYSPARAM_ORDER* order)
{
    assert(rail && order->workArea.right == 800 && order->workArea.bottom == 600);
    system_calls++;
    return CHANNEL_RC_OK;
}
static UINT execute(RailClientContext* rail, const RAIL_EXEC_ORDER* order)
{
    assert(rail && strcmp(order->RemoteApplicationProgram, "||notepad") == 0);
    execute_calls++;
    return CHANNEL_RC_OK;
}
static void event(int kind, const char* message, const char* fingerprint, void* data)
{
    (void)data;
    if (kind == RDP_EVENT_ERROR) { assert(message); error_calls++; }
    if (kind == RDP_EVENT_CERTIFICATE_REJECTED)
    {
        assert(message);
        certificate_rejections++;
        rejected_fingerprint_was_null = fingerprint == NULL;
        rejected_fingerprint[0] = '\0';
        if (fingerprint)
        {
            assert(strlen(fingerprint) < sizeof(rejected_fingerprint));
            memcpy(rejected_fingerprint, fingerprint, strlen(fingerprint) + 1);
        }
    }
}

static char* make_test_certificate(size_t* pem_length, char fingerprint[96])
{
    EVP_PKEY_CTX* key_context = EVP_PKEY_CTX_new_id(EVP_PKEY_RSA, NULL);
    EVP_PKEY* key = NULL;
    X509* certificate = NULL;
    X509_NAME* name = NULL;
    BIO* output = NULL;
    char* pem = NULL;
    unsigned char digest[EVP_MAX_MD_SIZE];
    unsigned int digest_length = 0;
    char* output_data = NULL;
    long output_length = 0;

    assert(key_context);
    assert(EVP_PKEY_keygen_init(key_context) == 1);
    assert(EVP_PKEY_CTX_set_rsa_keygen_bits(key_context, 2048) == 1);
    assert(EVP_PKEY_keygen(key_context, &key) == 1);
    certificate = X509_new();
    assert(certificate);
    assert(X509_set_version(certificate, 2) == 1);
    assert(ASN1_INTEGER_set(X509_get_serialNumber(certificate), 1) == 1);
    assert(X509_gmtime_adj(X509_get_notBefore(certificate), 0));
    assert(X509_gmtime_adj(X509_get_notAfter(certificate), 3600));
    assert(X509_set_pubkey(certificate, key) == 1);
    name = X509_get_subject_name(certificate);
    assert(name);
    assert(X509_NAME_add_entry_by_txt(name, "CN", MBSTRING_ASC,
                                      (const unsigned char*)"DeskFuse RDP test", -1, -1, 0) == 1);
    assert(X509_set_issuer_name(certificate, name) == 1);
    assert(X509_sign(certificate, key, EVP_sha256()) > 0);
    assert(X509_digest(certificate, EVP_sha256(), digest, &digest_length) == 1);
    assert(digest_length == 32);
    for (unsigned int index = 0; index < digest_length; index++)
        snprintf(fingerprint + index * 3, 4, index + 1 == digest_length ? "%02X" : "%02X:",
                 digest[index]);

    output = BIO_new(BIO_s_mem());
    assert(output);
    assert(PEM_write_bio_X509(output, certificate) == 1);
    output_length = BIO_get_mem_data(output, &output_data);
    assert(output_length > 0 && output_data);
    pem = malloc((size_t)output_length);
    assert(pem);
    memcpy(pem, output_data, (size_t)output_length);
    *pem_length = (size_t)output_length;

    BIO_free(output);
    X509_free(certificate);
    EVP_PKEY_free(key);
    EVP_PKEY_CTX_free(key_context);
    return pem;
}

static void reset_certificate_event(void)
{
    certificate_rejections = 0;
    rejected_fingerprint[0] = '\0';
    rejected_fingerprint_was_null = 0;
}
int main(void)
{
    assert(rdp_bridge_create(NULL, NULL, NULL, NULL, NULL, NULL) == NULL);
    rdp_bridge* bridge = rdp_bridge_create(
        "full address:s:127.0.0.1:3389\r\nremoteapplicationmode:i:1\r\n"
        "remoteapplicationprogram:s:||notepad\r\ndesktopwidth:i:800\r\ndesktopheight:i:600\r\n",
        "test", "not-a-real-password", NULL, event, NULL);
    assert(bridge);
    /* 关闭 RDP8 自动探测与心跳：隧道内只有单条 TCP 流，不需要它们。 */
    assert(!freerdp_settings_get_bool(bridge->context->settings, FreeRDP_NetworkAutoDetect));
    assert(!freerdp_settings_get_bool(bridge->context->settings, FreeRDP_SupportHeartbeatPdu));
    assert(!freerdp_settings_get_bool(bridge->context->settings, FreeRDP_DeviceRedirection));
    assert(freerdp_client_load_addins(bridge->context->channels, bridge->context->settings));
    /* rdpsnd 依赖 rdpdr：加载 addin 会强制打开设备重定向通道。没有配置任何
     * 驱动器、打印机或智能卡，所以通道存在但不转发设备。 */
    assert(freerdp_settings_get_bool(bridge->context->settings, FreeRDP_DeviceRedirection));
    assert(freerdp_settings_get_bool(bridge->context->settings, FreeRDP_RemoteApplicationMode));
    assert(!freerdp_settings_get_bool(bridge->context->settings, FreeRDP_SupportMultitransport));
    assert(freerdp_settings_get_uint32(bridge->context->settings, FreeRDP_MultitransportFlags) == 0);
    assert(freerdp_settings_get_bool(bridge->context->settings, FreeRDP_RedirectClipboard));
    assert(freerdp_settings_get_uint32(bridge->context->settings, FreeRDP_ClipboardFeatureMask) ==
           (CLIPRDR_FLAG_LOCAL_TO_REMOTE | CLIPRDR_FLAG_REMOTE_TO_LOCAL));
    assert(freerdp_settings_get_bool(bridge->context->settings, FreeRDP_AudioPlayback));
    assert(!freerdp_settings_get_bool(bridge->context->settings, FreeRDP_AudioCapture));
    rdp_bridge_set_audio(bridge, 0);
    assert(!freerdp_settings_get_bool(bridge->context->settings, FreeRDP_AudioPlayback));
    rdp_bridge_set_audio(bridge, 1);
    assert(freerdp_settings_get_bool(bridge->context->settings, FreeRDP_AudioPlayback));
    assert(!rdp_bridge_connected(bridge));

    /* Clipboard: text-only announcement, immediate and delayed answers. */
    rdp_bridge_set_clipboard_callback(bridge, clipboard_event, NULL);
    const uint32_t offered[] = { 13 };
    assert(rdp_bridge_clipboard_set_formats(bridge, offered, 1) == 0);
    assert(bridge->clip_format_count == 1 && bridge->clip_formats[0] == 13);
    bridge_drain_clipboard(bridge); /* announcer without a channel is dropped */
    assert(bridge->clip_command_count == 0);

    const uint8_t text[] = { 'h', 0, 'i', 0, 0, 0 };
    assert(rdp_bridge_clipboard_set_data(bridge, 13, text, sizeof(text)) == 0);
    assert(bridge->clip_data_count == 1);

    CliprdrClientContext clip = { 0 };
    clip.custom = bridge;
    clip.ClientFormatListResponse = clipboard_format_list_response;
    clip.ClientFormatDataResponse = clipboard_format_data_response;
    bridge->cliprdr = &clip;
    bridge->cliprdr_ready = 1;

    CLIPRDR_FORMAT_DATA_REQUEST request = { 0 };
    request.common.msgType = CB_FORMAT_DATA_REQUEST;
    request.requestedFormatId = 13;
    assert(bridge_clip_server_request(&clip, &request) == CHANNEL_RC_OK);
    assert(clipboard_response_flags == CB_RESPONSE_OK);
    assert(clipboard_response_length == sizeof(text));
    assert(memcmp(clipboard_response, text, sizeof(text)) == 0);
    assert(bridge->clip_pending_request == -1);

    request.requestedFormatId = 8;
    assert(bridge_clip_server_request(&clip, &request) == CHANNEL_RC_OK);
    assert(clipboard_requests == 1 && clipboard_requested_format == 8);
    assert(bridge->clip_pending_request == 8);

    const uint8_t clipboard_pixels[] = { 1, 2, 3, 4 };
    assert(rdp_bridge_clipboard_set_data(bridge, 8, clipboard_pixels, sizeof(clipboard_pixels)) == 0);
    assert(bridge->clip_pending_request == -1);
    assert(bridge->clip_command_count == 1);
    assert(bridge->clip_commands[0].op == CLIP_CMD_RESPOND);
    bridge_drain_clipboard(bridge);
    assert(bridge->clip_command_count == 0);

    assert(rdp_bridge_clipboard_request(bridge, 13) == 0);
    assert(bridge->clip_command_count == 1);
    assert(bridge->clip_commands[0].op == CLIP_CMD_REQUEST);
    bridge_drain_clipboard(bridge);

    CLIPRDR_FORMAT server_formats[] = {
        { .formatId = 13, .formatName = NULL },
        { .formatId = 8, .formatName = NULL },
    };
    CLIPRDR_FORMAT_LIST format_list = { 0 };
    format_list.numFormats = 2;
    format_list.formats = server_formats;
    assert(bridge_clip_server_formats(&clip, &format_list) == CHANNEL_RC_OK);
    assert(clipboard_list_responses == 1);
    assert(clipboard_format_lists == 1);

    assert(rdp_bridge_clipboard_set_formats(bridge, NULL, 0) == 0);
    assert(bridge->clip_format_count == 0 && bridge->clip_data_count == 0);
    bridge_drain_clipboard(bridge);
    bridge->cliprdr = NULL;
    bridge->cliprdr_ready = 0;
    assert(bridge_auth_failure(FREERDP_ERROR_AUTHENTICATION_FAILED));
    assert(bridge_auth_failure(FREERDP_ERROR_CONNECT_WRONG_PASSWORD));
    assert(bridge_auth_failure(FREERDP_ERROR_CONNECT_PASSWORD_MUST_CHANGE));
    assert(!bridge_auth_failure(FREERDP_ERROR_CONNECT_ACCOUNT_LOCKED_OUT));
    assert(!bridge_auth_failure(FREERDP_ERROR_CONNECT_TRANSPORT_FAILED));
    assert(bridge_pre_connect(bridge->instance));
    assert(freerdp_settings_get_bool(bridge->context->settings,
                                     FreeRDP_ExternalCertificateManagement));
    assert(!freerdp_settings_get_bool(bridge->context->settings, FreeRDP_IgnoreCertificate));
    assert(!freerdp_settings_get_bool(bridge->context->settings, FreeRDP_AutoAcceptCertificate));
    assert(bridge->instance->VerifyX509Certificate == bridge_verify_x509_certificate);
    assert(bridge->instance->Redirect == bridge_redirect);

    size_t pem_length = 0;
    char expected_fingerprint[96] = { 0 };
    char lowercase_fingerprint[96] = { 0 };
    char different_fingerprint[96] = { 0 };
    char* pem = make_test_certificate(&pem_length, expected_fingerprint);

    reset_certificate_event();
    assert(bridge->instance->VerifyX509Certificate(
               bridge->instance, (const BYTE*)pem, pem_length, "windows.test", 3389, 0) == 0);
    assert(certificate_rejections == 1 && !rejected_fingerprint_was_null);
    assert(strcasecmp(rejected_fingerprint, expected_fingerprint) == 0);
    assert(atomic_load(&bridge->certificate_rejected));

    bridge->certificate_fingerprint = _strdup(expected_fingerprint);
    assert(bridge->certificate_fingerprint);
    reset_certificate_event();
    assert(bridge->instance->VerifyX509Certificate(
               bridge->instance, (const BYTE*)pem, pem_length, "windows.test", 3389, 0) == 2);
    assert(certificate_rejections == 0);
    assert(!atomic_load(&bridge->certificate_rejected));

    memcpy(different_fingerprint, expected_fingerprint, sizeof(different_fingerprint));
    different_fingerprint[0] = different_fingerprint[0] == '0' ? '1' : '0';
    free(bridge->certificate_fingerprint);
    bridge->certificate_fingerprint = _strdup(different_fingerprint);
    reset_certificate_event();
    assert(bridge->instance->VerifyX509Certificate(
               bridge->instance, (const BYTE*)pem, pem_length, "windows.test", 3389, 0) == 0);
    assert(certificate_rejections == 1);
    assert(strcasecmp(rejected_fingerprint, expected_fingerprint) == 0);

    for (size_t index = 0; index < sizeof(lowercase_fingerprint); index++)
        lowercase_fingerprint[index] = (char)tolower((unsigned char)expected_fingerprint[index]);
    free(bridge->certificate_fingerprint);
    bridge->certificate_fingerprint = _strdup(lowercase_fingerprint);
    reset_certificate_event();
    assert(bridge->instance->VerifyX509Certificate(
               bridge->instance, (const BYTE*)pem, pem_length, "windows.test", 3389, 0) == 2);
    assert(certificate_rejections == 0);

    free(bridge->certificate_fingerprint);
    bridge->certificate_fingerprint = _strdup("00:11");
    reset_certificate_event();
    assert(bridge->instance->VerifyX509Certificate(
               bridge->instance, (const BYTE*)pem, pem_length, "windows.test", 3389, 0) == 0);
    assert(certificate_rejections == 1);

    free(bridge->certificate_fingerprint);
    bridge->certificate_fingerprint = _strdup(expected_fingerprint);
    reset_certificate_event();
    assert(bridge->instance->VerifyX509Certificate(
               bridge->instance, (const BYTE*)"not a certificate", strlen("not a certificate"),
               "windows.test", 3389, 0) == 0);
    assert(certificate_rejections == 1 && rejected_fingerprint_was_null);
    free(pem);

    RailClientContext rail = { 0 };
    railPlugin plugin = { 0 };
    plugin.rdpcontext = bridge->context;
    rail.handle = &plugin;
    rail.ClientInformation = information;
    rail.ClientSystemParam = system_parameters;
    rail.ClientExecute = execute;
    ChannelConnectedEventArgs connected = { 0 };
    connected.name = RAIL_SVC_CHANNEL_NAME;
    connected.pInterface = &rail;
    PubSub_OnChannelConnected(bridge->context->pubSub, bridge->context, &connected);
    assert(rail.custom == bridge && rail.ServerHandshake && rail.ServerHandshakeEx);
    assert(rail.ServerHandshake(&rail, NULL) == CHANNEL_RC_OK);
    assert(rail.ServerHandshakeEx(&rail, NULL) == CHANNEL_RC_OK);
    assert(information_calls == 1 && system_calls == 1 && execute_calls == 1);
    assert(rdp_bridge_execute(bridge, "notepad") == 0);
    assert(rdp_bridge_execute(bridge, "../bad") != 0);
    bridge_drain_execute(bridge);
    assert(execute_calls == 2);
    RAIL_EXEC_RESULT_ORDER result = { 0 };
    result.execResult = RAIL_EXEC_S_OK;
    assert(rail.ServerExecuteResult(&rail, &result) == CHANNEL_RC_OK);
    assert(error_calls == 0 && !atomic_load(&bridge->stop_requested));
    result.execResult = RAIL_EXEC_E_FAIL;
    assert(rail.ServerExecuteResult(&rail, &result) == CHANNEL_RC_OK);
    assert(error_calls == 1 && atomic_load(&bridge->stop_requested));
    ChannelDisconnectedEventArgs disconnected = { 0 };
    disconnected.name = RAIL_SVC_CHANNEL_NAME;
    disconnected.pInterface = &rail;
    PubSub_OnChannelDisconnected(bridge->context->pubSub, bridge->context, &disconnected);
    assert(!rail.custom && !rail.ServerHandshake);

    assert(bridge_post_connect(bridge->instance));
    assert(!freerdp_settings_get_bool(bridge->context->settings, FreeRDP_DeactivateClientDecoding));
    rdpGdi* gdi = bridge->context->gdi;
    memset(gdi->primary_buffer, 0x42, (size_t)gdi->stride * gdi->height);
    assert(bridge_end_paint(bridge->context));
    uint32_t w = 0, h = 0;
    assert(rdp_bridge_frame_size(bridge, &w, &h) == 0 && w == 800 && h == 600);
    size_t capacity = (size_t)w * h * 4;
    uint8_t* pixels = malloc(capacity);
    assert(pixels);
    assert(rdp_bridge_copy_frame(bridge, pixels, w * 4, capacity - 1, w, h) != 0);
    assert(rdp_bridge_copy_frame(bridge, pixels, w * 4, capacity, w + 1, h) != 0);
    assert(rdp_bridge_copy_frame(bridge, pixels, w * 4, capacity, w, h) == 0);
    assert(pixels[0] == 0x42 && pixels[capacity - 1] == 0x42);
    assert(rdp_bridge_frame_size(bridge, &w, &h) != 0);
    free(pixels);
    bridge_post_disconnect(bridge->instance);
    rdp_bridge_stop(bridge);
    rdp_bridge_stop(bridge);

    /* The bridge hands its QUIC pump end to the engine once; FreeRDP receives
     * the other end through the overridden TCPConnect and can never dial. */
    int pump = rdp_bridge_take_tunnel_fd(bridge);
    assert(pump >= 0);
    assert(rdp_bridge_take_tunnel_fd(bridge) == -1);
    int transport = bridge_tcp_connect(bridge->context, bridge->context->settings,
                                       "ignored.invalid", 3389, 0);
    assert(transport >= 0);
    assert(bridge_tcp_connect(bridge->context, bridge->context->settings,
                              "ignored.invalid", 3389, 0) == -1);
    char marker = 'x', echoed = 0;
    assert(write(pump, &marker, 1) == 1);
    assert(read(transport, &echoed, 1) == 1 && echoed == marker);
    close(pump);
    close(transport);

    rdp_bridge_free(bridge);
    puts("RDP bridge: certificate pinning, RAIL launch/error, clipboard text handshake, audio settings, auth classification, tunnel fd, GDI frame bounds and cleanup passed");
    return 0;
}
