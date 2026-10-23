/* Schmale C-Hülle um FreeRDP 3. Alle Aufrufe außer den Zwischenablage-
 * Rückrufen laufen im Faden des Workers; Zwischenablage-Rückrufe können aus
 * dem Faden des Kanals kommen. */

#include "rjrdp.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <freerdp/channels/channels.h>
#include <freerdp/client.h>
#include <freerdp/client/channels.h>
#include <freerdp/client/cliprdr.h>
#include <freerdp/client/disp.h>
#include <freerdp/client/rdpgfx.h>
#include <freerdp/gdi/gfx.h>
#include <freerdp/event.h>
#include <freerdp/freerdp.h>
#include <freerdp/gdi/gdi.h>
#include <freerdp/graphics.h>
#include <freerdp/codec/color.h>
#include <freerdp/input.h>
#include <freerdp/settings.h>
#include <winpr/synch.h>
#include <winpr/wlog.h>

struct rj_rdp
{
	rdpContext* context;
	rj_callbacks cb;
	CliprdrClientContext* cliprdr;
	DispClientContext* disp;
	uint32_t server_clip_flags;
	int gfx;
	char error[512];
};

typedef struct
{
	rdpClientContext common;
	rj_rdp* owner;
} rjContext;

static rj_rdp* owner_of(rdpContext* context)
{
	return context ? ((rjContext*)context)->owner : NULL;
}

/* ---------- Zwischenablage ---------- */

static UINT rj_cliprdr_server_caps(CliprdrClientContext* c, const CLIPRDR_CAPABILITIES* caps)
{
	rj_rdp* rdp = (rj_rdp*)c->custom;
	for (UINT32 i = 0; i < caps->cCapabilitiesSets; i++)
	{
		const CLIPRDR_CAPABILITY_SET* set = &caps->capabilitySets[i];
		if (set->capabilitySetType == CB_CAPSTYPE_GENERAL)
		{
			const CLIPRDR_GENERAL_CAPABILITY_SET* g = (const CLIPRDR_GENERAL_CAPABILITY_SET*)set;
			rdp->server_clip_flags = g->generalFlags;
		}
	}
	return CHANNEL_RC_OK;
}

static UINT rj_cliprdr_monitor_ready(CliprdrClientContext* c, const CLIPRDR_MONITOR_READY* m)
{
	(void)m;
	rj_rdp* rdp = (rj_rdp*)c->custom;
	CLIPRDR_GENERAL_CAPABILITY_SET g = { 0 };
	g.capabilitySetType = CB_CAPSTYPE_GENERAL;
	g.capabilitySetLength = CB_CAPSTYPE_GENERAL_LEN;
	g.version = CB_CAPS_VERSION_2;
	g.generalFlags = CB_USE_LONG_FORMAT_NAMES | CB_STREAM_FILECLIP_ENABLED |
	                 CB_FILECLIP_NO_FILE_PATHS | CB_HUGE_FILE_SUPPORT_ENABLED;
	CLIPRDR_CAPABILITIES caps = { 0 };
	caps.cCapabilitiesSets = 1;
	caps.capabilitySets = (CLIPRDR_CAPABILITY_SET*)&g;
	UINT rc = c->ClientCapabilities(c, &caps);
	if (rc != CHANNEL_RC_OK)
		return rc;
	CLIPRDR_FORMAT_LIST fl = { 0 };
	fl.common.msgType = CB_FORMAT_LIST;
	rc = c->ClientFormatList(c, &fl);
	if (rdp->cb.on_clip_ready)
		rdp->cb.on_clip_ready(rdp->cb.user, rdp->server_clip_flags);
	return rc;
}

static UINT rj_cliprdr_server_format_list(CliprdrClientContext* c, const CLIPRDR_FORMAT_LIST* fl)
{
	rj_rdp* rdp = (rj_rdp*)c->custom;
	CLIPRDR_FORMAT_LIST_RESPONSE resp = { 0 };
	resp.common.msgType = CB_FORMAT_LIST_RESPONSE;
	resp.common.msgFlags = CB_RESPONSE_OK;
	UINT rc = c->ClientFormatListResponse(c, &resp);
	if (rdp->cb.on_clip_format_list && fl->numFormats < 1024)
	{
		uint32_t ids[1024];
		const char* names[1024];
		for (UINT32 i = 0; i < fl->numFormats; i++)
		{
			ids[i] = fl->formats[i].formatId;
			names[i] = fl->formats[i].formatName ? fl->formats[i].formatName : "";
		}
		rdp->cb.on_clip_format_list(rdp->cb.user, fl->numFormats, ids, names);
	}
	return rc;
}

static UINT rj_cliprdr_server_format_list_response(CliprdrClientContext* c,
                                                   const CLIPRDR_FORMAT_LIST_RESPONSE* r)
{
	(void)c;
	(void)r;
	return CHANNEL_RC_OK;
}

static UINT rj_cliprdr_server_data_request(CliprdrClientContext* c,
                                           const CLIPRDR_FORMAT_DATA_REQUEST* r)
{
	rj_rdp* rdp = (rj_rdp*)c->custom;
	if (rdp->cb.on_clip_data_request)
		rdp->cb.on_clip_data_request(rdp->cb.user, r->requestedFormatId);
	return CHANNEL_RC_OK;
}

static UINT rj_cliprdr_server_data_response(CliprdrClientContext* c,
                                            const CLIPRDR_FORMAT_DATA_RESPONSE* r)
{
	rj_rdp* rdp = (rj_rdp*)c->custom;
	int ok = (r->common.msgFlags & CB_RESPONSE_OK) != 0;
	if (rdp->cb.on_clip_data_response)
		rdp->cb.on_clip_data_response(rdp->cb.user, ok, r->requestedFormatData,
		                              ok ? r->common.dataLen : 0);
	return CHANNEL_RC_OK;
}

static UINT rj_cliprdr_server_file_request(CliprdrClientContext* c,
                                           const CLIPRDR_FILE_CONTENTS_REQUEST* r)
{
	rj_rdp* rdp = (rj_rdp*)c->custom;
	if (rdp->cb.on_clip_file_request)
		rdp->cb.on_clip_file_request(rdp->cb.user, r->streamId, r->listIndex, r->dwFlags,
		                             ((uint64_t)r->nPositionHigh << 32) | r->nPositionLow,
		                             r->cbRequested);
	return CHANNEL_RC_OK;
}

static UINT rj_cliprdr_server_file_response(CliprdrClientContext* c,
                                            const CLIPRDR_FILE_CONTENTS_RESPONSE* r)
{
	rj_rdp* rdp = (rj_rdp*)c->custom;
	int ok = (r->common.msgFlags & CB_RESPONSE_OK) != 0;
	if (rdp->cb.on_clip_file_response)
		rdp->cb.on_clip_file_response(rdp->cb.user, r->streamId, ok, r->requestedData,
		                              ok ? r->cbRequested : 0);
	return CHANNEL_RC_OK;
}

/* Diagnoseschalter (nur Umgebungsvariablen, zum Eingrenzen von Verbindungsfehlern):
 *   RJ_RDP_NODISP=1        Display Control und dynamische Auflösung nicht anbieten
 *   RJ_RDP_GFX=1           Grafikkanal (RDPGFX) anbieten
 *   RJ_RDP_SEC=nla|tls|rdp nur diesen Sicherheitsmodus erlauben
 *   RJ_RDP_NOAUTODETECT=1  Netzwerkerkennung (Auto-Detect) nicht anbieten
 *   RJ_RDP_NOHEARTBEAT=1   Heartbeat-PDU nicht anbieten
 *   RJ_RDP_NOSKIPJOIN=1    „Kanalbeitritt überspringen“ nicht anbieten
 *   RJ_RDP_LEGACY=1        alle drei zugleich (klassischer Ablauf) */
static int rj_env_is(const char* name, const char* value)
{
	const char* v = getenv(name);
	return v && strcmp(v, value) == 0;
}

static void rj_channel_connected(void* ctx, const ChannelConnectedEventArgs* e)
{
	rj_rdp* rdp = owner_of((rdpContext*)ctx);
	if (!rdp)
		return;
	if (strcmp(e->name, CLIPRDR_SVC_CHANNEL_NAME) == 0)
	{
		CliprdrClientContext* c = (CliprdrClientContext*)e->pInterface;
		c->custom = rdp;
		c->ServerCapabilities = rj_cliprdr_server_caps;
		c->MonitorReady = rj_cliprdr_monitor_ready;
		c->ServerFormatList = rj_cliprdr_server_format_list;
		c->ServerFormatListResponse = rj_cliprdr_server_format_list_response;
		c->ServerFormatDataRequest = rj_cliprdr_server_data_request;
		c->ServerFormatDataResponse = rj_cliprdr_server_data_response;
		c->ServerFileContentsRequest = rj_cliprdr_server_file_request;
		c->ServerFileContentsResponse = rj_cliprdr_server_file_response;
		rdp->cliprdr = c;
	}
	else if (strcmp(e->name, DISP_DVC_CHANNEL_NAME) == 0)
	{
		rdp->disp = (DispClientContext*)e->pInterface;
	}
	else if (strcmp(e->name, RDPGFX_DVC_CHANNEL_NAME) == 0 && rdp->gfx)
	{
		gdi_graphics_pipeline_init(((rdpContext*)ctx)->gdi, (RdpgfxClientContext*)e->pInterface);
	}
}

static void rj_channel_disconnected(void* ctx, const ChannelDisconnectedEventArgs* e)
{
	rj_rdp* rdp = owner_of((rdpContext*)ctx);
	if (!rdp)
		return;
	if (strcmp(e->name, CLIPRDR_SVC_CHANNEL_NAME) == 0)
		rdp->cliprdr = NULL;
	else if (strcmp(e->name, DISP_DVC_CHANNEL_NAME) == 0)
		rdp->disp = NULL;
	else if (strcmp(e->name, RDPGFX_DVC_CHANNEL_NAME) == 0 && rdp->gfx)
		gdi_graphics_pipeline_uninit(((rdpContext*)ctx)->gdi, (RdpgfxClientContext*)e->pInterface);
}

/* ---------- Bild ---------- */

static BOOL rj_begin_paint(rdpContext* context)
{
	rdpGdi* gdi = context->gdi;
	gdi->primary->hdc->hwnd->invalid->null = TRUE;
	return TRUE;
}

static BOOL rj_end_paint(rdpContext* context)
{
	rdpGdi* gdi = context->gdi;
	rj_rdp* rdp = owner_of(context);
	HGDI_RGN inv = gdi->primary->hdc->hwnd->invalid;
	if (inv->null || !rdp || !rdp->cb.on_frame)
		return TRUE;
	rdp->cb.on_frame(rdp->cb.user, gdi->primary_buffer, (uint32_t)gdi->width,
	                 (uint32_t)gdi->height, gdi->stride, inv->x, inv->y, inv->w, inv->h);
	return TRUE;
}

static BOOL rj_desktop_resize(rdpContext* context)
{
	rdpSettings* s = context->settings;
	UINT32 w = freerdp_settings_get_uint32(s, FreeRDP_DesktopWidth);
	UINT32 h = freerdp_settings_get_uint32(s, FreeRDP_DesktopHeight);
	if (!gdi_resize(context->gdi, w, h))
		return FALSE;
	rj_rdp* rdp = owner_of(context);
	if (rdp && rdp->cb.on_desktop_resize)
		rdp->cb.on_desktop_resize(rdp->cb.user, w, h);
	return TRUE;
}

/* ---------- Verbindung ---------- */

static BOOL rj_pre_connect(freerdp* instance)
{
	rdpContext* ctx = instance->context;
	PubSub_SubscribeChannelConnected(ctx->pubSub, rj_channel_connected);
	PubSub_SubscribeChannelDisconnected(ctx->pubSub, rj_channel_disconnected);
	return TRUE;
}

/* ---------- Mauszeiger der Gegenstelle ---------- */

typedef struct
{
	rdpPointer pointer;
	BYTE* data; /* BGRA32, width × height */
} rjPointer;

static BOOL rj_pointer_new(rdpContext* context, rdpPointer* pointer)
{
	rjPointer* p = (rjPointer*)pointer;
	if (pointer->width == 0 || pointer->height == 0 || pointer->width > 384 || pointer->height > 384)
		return TRUE;
	p->data = calloc((size_t)pointer->width * pointer->height, 4);
	if (!p->data)
		return FALSE;
	rdpGdi* gdi = context->gdi;
	if (!freerdp_image_copy_from_pointer_data(p->data, PIXEL_FORMAT_BGRA32, 0, 0, 0, pointer->width, pointer->height,
	                                          pointer->xorMaskData, pointer->lengthXorMask, pointer->andMaskData,
	                                          pointer->lengthAndMask, pointer->xorBpp, gdi ? &gdi->palette : NULL))
	{
		free(p->data);
		p->data = NULL;
		return FALSE;
	}
	return TRUE;
}

static void rj_pointer_free(rdpContext* context, rdpPointer* pointer)
{
	(void)context;
	rjPointer* p = (rjPointer*)pointer;
	free(p->data);
	p->data = NULL;
}

static BOOL rj_pointer_set(rdpContext* context, rdpPointer* pointer)
{
	rj_rdp* rdp = owner_of(context);
	rjPointer* p = (rjPointer*)pointer;
	if (rdp && rdp->cb.on_pointer)
	{
		if (p->data)
			rdp->cb.on_pointer(rdp->cb.user, 2, p->data, pointer->width, pointer->height, pointer->xPos, pointer->yPos);
		else
			rdp->cb.on_pointer(rdp->cb.user, 0, NULL, 0, 0, 0, 0);
	}
	return TRUE;
}

static BOOL rj_pointer_set_null(rdpContext* context)
{
	rj_rdp* rdp = owner_of(context);
	if (rdp && rdp->cb.on_pointer)
		rdp->cb.on_pointer(rdp->cb.user, 1, NULL, 0, 0, 0, 0);
	return TRUE;
}

static BOOL rj_pointer_set_default(rdpContext* context)
{
	rj_rdp* rdp = owner_of(context);
	if (rdp && rdp->cb.on_pointer)
		rdp->cb.on_pointer(rdp->cb.user, 0, NULL, 0, 0, 0, 0);
	return TRUE;
}

static BOOL rj_pointer_set_position(rdpContext* context, UINT32 x, UINT32 y)
{
	(void)context;
	(void)x;
	(void)y;
	return TRUE;
}

static BOOL rj_post_connect(freerdp* instance)
{
	if (!gdi_init(instance, PIXEL_FORMAT_BGRA32))
		return FALSE;
	rdpPointer pointer = { 0 };
	pointer.size = sizeof(rjPointer);
	pointer.New = rj_pointer_new;
	pointer.Free = rj_pointer_free;
	pointer.Set = rj_pointer_set;
	pointer.SetNull = rj_pointer_set_null;
	pointer.SetDefault = rj_pointer_set_default;
	pointer.SetPosition = rj_pointer_set_position;
	graphics_register_pointer(instance->context->graphics, &pointer);
	rdpUpdate* up = instance->context->update;
	up->BeginPaint = rj_begin_paint;
	up->EndPaint = rj_end_paint;
	up->DesktopResize = rj_desktop_resize;
	return TRUE;
}

static void rj_post_disconnect(freerdp* instance)
{
	gdi_free(instance);
}

static BOOL rj_authenticate(freerdp* instance, char** username, char** password, char** domain,
                            rdp_auth_reason reason)
{
	(void)instance;
	(void)reason;
	/* Keine Rückfrage: fehlen Zugangsdaten, schlägt die Anmeldung fehl. */
	return (username && *username && password && *password) ? TRUE : FALSE;
	(void)domain;
}

static DWORD rj_verify_cert(freerdp* instance, const char* host, UINT16 port,
                            const char* common_name, const char* subject, const char* issuer,
                            const char* fingerprint, DWORD flags)
{
	(void)common_name;
	(void)flags;
	rj_rdp* rdp = owner_of(instance->context);
	if (!rdp || !rdp->cb.verify_certificate)
		return 0;
	return rdp->cb.verify_certificate(rdp->cb.user, host, port, subject, issuer, fingerprint, NULL);
}

static DWORD rj_verify_changed_cert(freerdp* instance, const char* host, UINT16 port,
                                    const char* common_name, const char* subject,
                                    const char* issuer, const char* new_fingerprint,
                                    const char* old_subject, const char* old_issuer,
                                    const char* old_fingerprint, DWORD flags)
{
	(void)common_name;
	(void)old_subject;
	(void)old_issuer;
	(void)flags;
	rj_rdp* rdp = owner_of(instance->context);
	if (!rdp || !rdp->cb.verify_certificate)
		return 0;
	return rdp->cb.verify_certificate(rdp->cb.user, host, port, subject, issuer, new_fingerprint,
	                                  old_fingerprint ? old_fingerprint : "");
}

static BOOL rj_client_new(freerdp* instance, rdpContext* context)
{
	(void)context;
	instance->PreConnect = rj_pre_connect;
	instance->PostConnect = rj_post_connect;
	instance->PostDisconnect = rj_post_disconnect;
	instance->AuthenticateEx = rj_authenticate;
	instance->VerifyCertificateEx = rj_verify_cert;
	instance->VerifyChangedCertificateEx = rj_verify_changed_cert;
	return TRUE;
}

static void rj_client_free(freerdp* instance, rdpContext* context)
{
	(void)instance;
	(void)context;
}

rj_rdp* rj_rdp_new(const rj_callbacks* cb)
{
	rj_rdp* rdp = calloc(1, sizeof(rj_rdp));
	if (!rdp)
		return NULL;
	rdp->cb = *cb;
	RDP_CLIENT_ENTRY_POINTS ep = { 0 };
	ep.Version = RDP_CLIENT_INTERFACE_VERSION;
	ep.Size = sizeof(RDP_CLIENT_ENTRY_POINTS_V1);
	ep.ContextSize = sizeof(rjContext);
	ep.ClientNew = rj_client_new;
	ep.ClientFree = rj_client_free;
	rdp->context = freerdp_client_context_new(&ep);
	if (!rdp->context)
	{
		free(rdp);
		return NULL;
	}
	((rjContext*)rdp->context)->owner = rdp;
	return rdp;
}

void rj_rdp_free(rj_rdp* rdp)
{
	if (!rdp)
		return;
	if (rdp->context)
	{
		((rjContext*)rdp->context)->owner = NULL;
		freerdp_client_context_free(rdp->context);
	}
	free(rdp);
}

static void set_error(rj_rdp* rdp, const char* what)
{
	UINT32 code = freerdp_get_last_error(rdp->context);
	snprintf(rdp->error, sizeof(rdp->error), "%s: %s (%s)", what,
	         freerdp_get_last_error_string(code), freerdp_get_last_error_name(code));
}

const char* rj_rdp_last_error(rj_rdp* rdp)
{
	return rdp ? rdp->error : "";
}

uint32_t rj_rdp_error_info(rj_rdp* rdp)
{
	return (rdp && rdp->context && rdp->context->instance) ? freerdp_error_info(rdp->context->instance) : 0;
}

/* Lokales Konto: ohne Domäne (leer oder ".") und ohne "@" bzw. "\\" im Benutzernamen.
 * Dafür gibt es kein Kerberos; der Versuch kostet nur ein bis zwei Sekunden. */
static BOOL rj_is_local_account(const rj_params* p)
{
	const char* d = p->domain;
	if (d && *d && strcmp(d, ".") != 0)
		return FALSE;
	return (p->username && !strchr(p->username, '@') && !strchr(p->username, '\\')) ? TRUE : FALSE;
}

int rj_rdp_connect(rj_rdp* rdp, const rj_params* p)
{
	rdpSettings* s = rdp->context->settings;
	BOOL ok = TRUE;
	ok &= freerdp_settings_set_string(s, FreeRDP_ServerHostname, p->host);
	ok &= freerdp_settings_set_uint32(s, FreeRDP_ServerPort, p->port);
	ok &= freerdp_settings_set_string(s, FreeRDP_Username, p->username);
	if (p->domain && *p->domain)
		ok &= freerdp_settings_set_string(s, FreeRDP_Domain, p->domain);
	ok &= freerdp_settings_set_string(s, FreeRDP_Password, p->password);
	/* Startgröße: Breite gerade (wie beim späteren Größenwechsel; GNOME-Remote-Desktop und andere Server
	 * lehnen ungerade Breiten ab). Die angeforderte Größe steht zur Diagnose im Protokoll des Workers. */
	{
		UINT32 dw = p->width & ~1u;
		UINT32 dh = p->height;
		if (dw < 200)
			dw = 200;
		if (dh < 200)
			dh = 200;
		fprintf(stderr, "[remotejoe-rdp] Startgröße %ux%u (angefordert %ux%u)\n", dw, dh, p->width, p->height);
		ok &= freerdp_settings_set_uint32(s, FreeRDP_DesktopWidth, dw);
		ok &= freerdp_settings_set_uint32(s, FreeRDP_DesktopHeight, dh);
	}
	ok &= freerdp_settings_set_uint32(s, FreeRDP_ColorDepth, 32);
	ok &= freerdp_settings_set_bool(s, FreeRDP_RedirectClipboard, p->clipboard ? TRUE : FALSE);
	ok &= freerdp_settings_set_bool(s, FreeRDP_SupportDisplayControl, TRUE);
	ok &= freerdp_settings_set_bool(s, FreeRDP_DynamicResolutionUpdate, TRUE);
	ok &= freerdp_settings_set_bool(s, FreeRDP_SupportGraphicsPipeline, FALSE);
	ok &= freerdp_settings_set_bool(s, FreeRDP_AutoReconnectionEnabled, FALSE);
	ok &= freerdp_settings_set_bool(s, FreeRDP_IgnoreCertificate, p->ignore_certificate ? TRUE : FALSE);
	ok &= freerdp_settings_set_uint32(s, FreeRDP_TcpConnectTimeout, p->timeout_ms);
	if (p->keyboard_layout)
		ok &= freerdp_settings_set_uint32(s, FreeRDP_KeyboardLayout, p->keyboard_layout);
	if (p->config_path && *p->config_path)
		ok &= freerdp_settings_set_string(s, FreeRDP_ConfigPath, p->config_path);
	BOOL nla = TRUE, tls = TRUE, rdpsec = TRUE;
	switch (p->security)
	{
		case RJ_SEC_NLA:
			tls = FALSE;
			rdpsec = FALSE;
			break;
		case RJ_SEC_TLS:
			nla = FALSE;
			rdpsec = FALSE;
			break;
		case RJ_SEC_RDP:
			nla = FALSE;
			tls = FALSE;
			break;
		default:
			break;
	}
	/* Diagnose: Umgebungsvariablen überschreiben einzelne Voreinstellungen. */
	if (rj_env_is("RJ_RDP_NODISP", "1"))
	{
		ok &= freerdp_settings_set_bool(s, FreeRDP_SupportDisplayControl, FALSE);
		ok &= freerdp_settings_set_bool(s, FreeRDP_DynamicResolutionUpdate, FALSE);
		fprintf(stderr, "[remotejoe-rdp] Diagnose: Display Control aus\n");
	}
	if (rj_env_is("RJ_RDP_NOAUTODETECT", "1") || rj_env_is("RJ_RDP_LEGACY", "1"))
	{
		ok &= freerdp_settings_set_bool(s, FreeRDP_NetworkAutoDetect, FALSE);
		ok &= freerdp_settings_set_uint32(s, FreeRDP_ConnectionType, CONNECTION_TYPE_LAN);
		fprintf(stderr, "[remotejoe-rdp] Diagnose: Auto-Detect aus\n");
	}
	if (rj_env_is("RJ_RDP_NOHEARTBEAT", "1") || rj_env_is("RJ_RDP_LEGACY", "1"))
	{
		ok &= freerdp_settings_set_bool(s, FreeRDP_SupportHeartbeatPdu, FALSE);
		fprintf(stderr, "[remotejoe-rdp] Diagnose: Heartbeat aus\n");
	}
	if (rj_env_is("RJ_RDP_NOSKIPJOIN", "1") || rj_env_is("RJ_RDP_LEGACY", "1"))
	{
		ok &= freerdp_settings_set_bool(s, FreeRDP_SupportSkipChannelJoin, FALSE);
		fprintf(stderr, "[remotejoe-rdp] Diagnose: Kanalbeitritt wird nicht übersprungen\n");
	}
	rdp->gfx = p->gfx || rj_env_is("RJ_RDP_GFX", "1");
	if (rdp->gfx)
	{
		ok &= freerdp_settings_set_bool(s, FreeRDP_SupportGraphicsPipeline, TRUE);
		ok &= freerdp_settings_set_bool(s, FreeRDP_GfxH264, FALSE);
		ok &= freerdp_settings_set_bool(s, FreeRDP_GfxAVC444, FALSE);
		ok &= freerdp_settings_set_bool(s, FreeRDP_GfxAVC444v2, FALSE);
		fprintf(stderr, "[remotejoe-rdp] Grafikkanal an\n");
	}
	if (rj_env_is("RJ_RDP_SEC", "nla") || rj_env_is("RJ_RDP_SEC", "tls") || rj_env_is("RJ_RDP_SEC", "rdp"))
	{
		nla = rj_env_is("RJ_RDP_SEC", "nla");
		tls = rj_env_is("RJ_RDP_SEC", "tls");
		rdpsec = rj_env_is("RJ_RDP_SEC", "rdp");
		fprintf(stderr, "[remotejoe-rdp] Diagnose: Sicherheitsmodus %s\n", getenv("RJ_RDP_SEC"));
	}
	if (rj_is_local_account(p))
	{
		ok &= freerdp_settings_set_string(s, FreeRDP_AuthenticationPackageList, "!kerberos");
		/* Bei lokalen Konten probiert die Bibliothek Kerberos trotzdem kurz an und protokolliert dann
		 * „Configuration file does not specify default realm“ als Fehler, obwohl danach NTLM sauber
		 * anmeldet. Diese eine Meldung wird abgeschaltet; alle anderen bleiben. */
		WLog_SetLogLevel(WLog_Get("com.winpr.sspi.Kerberos"), WLOG_OFF);
	}
	ok &= freerdp_settings_set_bool(s, FreeRDP_NlaSecurity, nla);
	ok &= freerdp_settings_set_bool(s, FreeRDP_TlsSecurity, tls);
	ok &= freerdp_settings_set_bool(s, FreeRDP_RdpSecurity, rdpsec);
	/* Prüfung der Zugangsdaten: nur die NLA-Anmeldung, danach Ende ohne Sitzung (wie xfreerdp /auth-only). */
	if (p->auth_only)
		ok &= freerdp_settings_set_bool(s, FreeRDP_AuthenticationOnly, TRUE);
	if (!ok)
	{
		snprintf(rdp->error, sizeof(rdp->error), "Einstellungen ungültig");
		return -1;
	}
	if (!freerdp_connect(rdp->context->instance))
	{
		set_error(rdp, "Verbindung fehlgeschlagen");
		return -1;
	}
	return 0;
}

int rj_rdp_poll(rj_rdp* rdp, uint32_t timeout_ms)
{
	rdpContext* ctx = rdp->context;
	if (freerdp_shall_disconnect_context(ctx))
		return -1;
	HANDLE handles[MAXIMUM_WAIT_OBJECTS] = { 0 };
	DWORD n = freerdp_get_event_handles(ctx, handles, ARRAYSIZE(handles));
	if (n == 0)
	{
		set_error(rdp, "keine Ereignisse");
		return -1;
	}
	WaitForMultipleObjects(n, handles, FALSE, timeout_ms);
	if (!freerdp_check_event_handles(ctx))
	{
		set_error(rdp, "Verbindung getrennt");
		return -1;
	}
	return freerdp_shall_disconnect_context(ctx) ? -1 : 0;
}

void rj_rdp_disconnect(rj_rdp* rdp)
{
	if (rdp && rdp->context)
		freerdp_disconnect(rdp->context->instance);
}

int rj_rdp_send_unicode(rj_rdp* rdp, uint16_t code, int down)
{
	return freerdp_input_send_unicode_keyboard_event(rdp->context->input,
	                                                 down ? 0 : KBD_FLAGS_RELEASE, code)
	           ? 0
	           : -1;
}

int rj_rdp_send_scancode(rj_rdp* rdp, uint32_t scancode, int down)
{
	return freerdp_input_send_keyboard_event_ex(rdp->context->input, down ? TRUE : FALSE, FALSE,
	                                            scancode)
	           ? 0
	           : -1;
}

int rj_rdp_send_mouse(rj_rdp* rdp, uint16_t flags, uint16_t x, uint16_t y)
{
	return freerdp_input_send_mouse_event(rdp->context->input, flags, x, y) ? 0 : -1;
}

int rj_rdp_resize(rj_rdp* rdp, uint32_t width, uint32_t height)
{
	if (!rdp->disp)
		return -1;
	DISPLAY_CONTROL_MONITOR_LAYOUT l = { 0 };
	l.Flags = DISPLAY_CONTROL_MONITOR_PRIMARY;
	/* MS-RDPEDISP / GNOME-Remote-Desktop: Breite gerade, Breite und Höhe 200 bis 8192. */
	if (width < 200)
		width = 200;
	if (height < 200)
		height = 200;
	if (width > 8192)
		width = 8192;
	if (height > 8192)
		height = 8192;
	l.Width = width & ~1u;
	l.Height = height;
	l.DesktopScaleFactor = 100;
	l.DeviceScaleFactor = 100;
	return rdp->disp->SendMonitorLayout(rdp->disp, 1, &l) == CHANNEL_RC_OK ? 0 : -1;
}

int rj_clip_send_format_list(rj_rdp* rdp, uint32_t n, const uint32_t* ids, const char* const* names)
{
	CliprdrClientContext* c = rdp->cliprdr;
	if (!c || n > 64)
		return -1;
	CLIPRDR_FORMAT formats[64] = { 0 };
	for (uint32_t i = 0; i < n; i++)
	{
		formats[i].formatId = ids[i];
		formats[i].formatName = (names && names[i] && *names[i]) ? (char*)names[i] : NULL;
	}
	CLIPRDR_FORMAT_LIST fl = { 0 };
	fl.common.msgType = CB_FORMAT_LIST;
	fl.numFormats = n;
	fl.formats = formats;
	return c->ClientFormatList(c, &fl) == CHANNEL_RC_OK ? 0 : -1;
}

int rj_clip_request_data(rj_rdp* rdp, uint32_t format_id)
{
	CliprdrClientContext* c = rdp->cliprdr;
	if (!c)
		return -1;
	CLIPRDR_FORMAT_DATA_REQUEST r = { 0 };
	r.common.msgType = CB_FORMAT_DATA_REQUEST;
	r.requestedFormatId = format_id;
	return c->ClientFormatDataRequest(c, &r) == CHANNEL_RC_OK ? 0 : -1;
}

int rj_clip_send_data_response(rj_rdp* rdp, int ok, const uint8_t* data, uint32_t len)
{
	CliprdrClientContext* c = rdp->cliprdr;
	if (!c)
		return -1;
	CLIPRDR_FORMAT_DATA_RESPONSE r = { 0 };
	r.common.msgType = CB_FORMAT_DATA_RESPONSE;
	r.common.msgFlags = ok ? CB_RESPONSE_OK : CB_RESPONSE_FAIL;
	r.common.dataLen = ok ? len : 0;
	r.requestedFormatData = ok ? data : NULL;
	return c->ClientFormatDataResponse(c, &r) == CHANNEL_RC_OK ? 0 : -1;
}

int rj_clip_request_file(rj_rdp* rdp, uint32_t stream_id, uint32_t list_index, uint32_t flags,
                         uint64_t position, uint32_t requested)
{
	CliprdrClientContext* c = rdp->cliprdr;
	if (!c)
		return -1;
	CLIPRDR_FILE_CONTENTS_REQUEST r = { 0 };
	r.common.msgType = CB_FILECONTENTS_REQUEST;
	r.streamId = stream_id;
	r.listIndex = list_index;
	r.dwFlags = flags;
	r.nPositionLow = (UINT32)(position & 0xFFFFFFFFu);
	r.nPositionHigh = (UINT32)(position >> 32);
	r.cbRequested = requested;
	return c->ClientFileContentsRequest(c, &r) == CHANNEL_RC_OK ? 0 : -1;
}

int rj_clip_send_file_response(rj_rdp* rdp, uint32_t stream_id, int ok, const uint8_t* data,
                               uint32_t len)
{
	CliprdrClientContext* c = rdp->cliprdr;
	if (!c)
		return -1;
	CLIPRDR_FILE_CONTENTS_RESPONSE r = { 0 };
	r.common.msgType = CB_FILECONTENTS_RESPONSE;
	r.common.msgFlags = ok ? CB_RESPONSE_OK : CB_RESPONSE_FAIL;
	r.common.dataLen = ok ? len + 4 : 4;
	r.streamId = stream_id;
	r.cbRequested = ok ? len : 0;
	r.requestedData = ok ? data : NULL;
	return c->ClientFileContentsResponse(c, &r) == CHANNEL_RC_OK ? 0 : -1;
}
