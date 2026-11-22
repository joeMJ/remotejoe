// Lader der Erweiterungs-Oberflächen. Der Kern (Rust) hat die Bibliotheken nach Signaturprüfung geöffnet und nennt
// die Adressen ihrer Einsprungpunkte `rj_plugin_ui`; hier wird je Bibliothek die Oberfläche angelegt, sofern die
// Module freigeschaltet sind. Der Kern aus Sicht der Erweiterung ist `RjHostBackend` (Adapter über RjBackend).

#include "rjpluginui.h"

#include "../mainwindow_p.h"
#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QtDebug>

namespace {

class BackendHost : public RjHostBackend
{
public:
	explicit BackendHost(RjBackend* b) : m_b(b) {}
	QString text(const QString& key) const override { return m_b->text(key); }
	QString edit(const QString& cmdJson) override { return m_b->edit(cmdJson); }
	bool moduleActive(const QString& id) const override { return m_b->moduleActive(id); }
	bool modulesActive() const override { return m_b->modulesActive(); }
	QIcon icon(const QString& kind, const QString& light, bool folder, int session) const override { return rjmw::icon(kind, light, folder, session); }

private:
	RjBackend* m_b;
};

using UiEntry = void* (*)(void* backend, int hostUiVersion);

} // namespace

RjHostBackend* rjHostBackend(RjBackend* backend)
{
	static BackendHost* host = nullptr;
	if (!host)
		host = new BackendHost(backend);
	return host;
}

const std::vector<RjPluginUi*>& rjPluginUis(RjBackend* backend)
{
	static std::vector<RjPluginUi*> list;
	static bool loaded = false;
	if (!loaded)
	{
		loaded = true;
		if (backend->modulesActive())
		{
			const QJsonArray entries = QJsonDocument::fromJson(backend->pluginUiEntries().toUtf8()).array();
			for (const QJsonValue& v : entries)
			{
				const auto addr = static_cast<quintptr>(v.toDouble());
				if (!addr)
					continue;
				auto entry = reinterpret_cast<UiEntry>(addr);
				if (auto* p = static_cast<RjPluginUi*>(entry(rjHostBackend(backend), RJ_HOST_UI_VERSION)))
					list.push_back(p);
				else
					qWarning("remotejoe: Oberfläche einer Erweiterung passt nicht zur Fassung %d der Host-Schnittstelle", RJ_HOST_UI_VERSION);
			}
		}
	}
	return list;
}

QString rjPluginText(RjBackend* backend, const char* key)
{
	for (RjPluginUi* p : rjPluginUis(backend))
		if (const QString t = p->textFor(key); !t.isEmpty())
			return t;
	return backend->text(QString::fromLatin1(key));
}
