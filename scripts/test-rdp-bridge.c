/* White-box native regression tests against the pinned FreeRDP headers/library. */
#include <assert.h>
#include "../apps/deskfuse-macos/RemoteApplication/Sources/DeskFuseRemoteApplication/RdpBridge/RdpBridge.c"
#include "rail_main.h"

static unsigned information_calls, system_calls, execute_calls, error_calls;
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
static void event(int kind, const char* message, void* data)
{
    (void)data;
    if (kind == RDP_EVENT_ERROR) { assert(message); error_calls++; }
}
int main(void)
{
    assert(rdp_bridge_create(NULL, NULL, NULL, NULL, NULL) == NULL);
    rdp_bridge* bridge = rdp_bridge_create(
        "full address:s:127.0.0.1:3389\r\nremoteapplicationmode:i:1\r\n"
        "remoteapplicationprogram:s:||notepad\r\ndesktopwidth:i:800\r\ndesktopheight:i:600\r\n",
        "test", "not-a-real-password", event, NULL);
    assert(bridge);
    assert(freerdp_settings_get_bool(bridge->context->settings, FreeRDP_RemoteApplicationMode));
    assert(!rdp_bridge_connected(bridge));
    assert(bridge_pre_connect(bridge->instance));
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
    assert(information_calls == 2 && system_calls == 2 && execute_calls == 2);
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
    rdp_bridge_free(bridge);
    puts("RDP bridge: parse, RAIL launch/error, GDI frame bounds and cleanup passed");
    return 0;
}
