// Oberfläche einer Erweiterung (Plugin): Beiträge zum Hauptfenster, zur Objektübersicht, zu den Einstellungen und
// zu den Eigenschaften. Der Kern kennt nur diese Schnittstelle; die Erweiterung kennt vom Kern nur rjhost.h.
// Die Bibliothek der Erweiterung führt `extern "C" RjPluginUi* rj_plugin_ui_create(RjHostBackend*, int hostUiVersion)`
// aus (null, wenn die Fassung nicht passt); der Kern ruft sie nach geprüfter Signatur auf.
#pragma once

#include "rjhost.h"

#include <QtCore/QJsonArray>
#include <QtCore/QJsonObject>
#include <QtCore/QObject>
#include <QtCore/QString>
#include <QtWidgets/QWidget>

#include <functional>
#include <vector>

class QVBoxLayout;
class QWidget;

/// Beitrag zur Objektübersicht eines Ziels; je Übersicht ein Objekt, das der Tab besitzt.
class RjOverviewPart : public QObject
{
public:
	using QObject::QObject;
	/// Karten nach den Kennzahlen bzw. nach den Prüfungen anlegen (zunächst verborgen).
	virtual void createAfterInventory(QVBoxLayout* lay, QWidget* page) { Q_UNUSED(lay) Q_UNUSED(page) }
	virtual void createAfterChecks(QVBoxLayout* lay, QWidget* page) { Q_UNUSED(lay) Q_UNUSED(page) }
	/// Antwort „overview“ eingetroffen (beim Laden und beim Auffrischen); `guest`: ein Gast eines Hypervisors.
	virtual void overviewLoaded(const QJsonObject& r, bool guest) { Q_UNUSED(r) Q_UNUSED(guest) }
	/// Ergebnis eines Hintergrundauftrags; `true` = erledigt.
	virtual bool handleResult(const QString& kind, const QJsonObject& r) { Q_UNUSED(kind) Q_UNUSED(r) return false; }
};

/// Beitrag zu den Einstellungen (Bereiche unter „Erweiterungen“).
class RjSettingsPart : public QObject
{
public:
	using QObject::QObject;
	virtual void addAreas() {}
	virtual void load(const QJsonObject& settings) { Q_UNUSED(settings) }
	virtual void summaries(const std::function<void(const char*, const QString&)>& set) { Q_UNUSED(set) }
};

/// Beitrag zu den Eigenschaften eines Eintrags.
class RjEditorPart : public QObject
{
public:
	using QObject::QObject;
	/// Seiten nach „Allgemein“ anlegen (über RjHostProperties::addPluginPage).
	virtual void addPages() {}
	/// Kurzfassung eines eigenen Bereichs; leer = kein eigener Bereich.
	virtual QString summary(const QString& key, const QJsonObject& d) const { Q_UNUSED(key) Q_UNUSED(d) return {}; }
	/// Geänderte Felder in die Speicheranfrage eintragen.
	virtual void save(QJsonObject& f) const { Q_UNUSED(f) }
};

/// Die Erweiterung selbst: liefert die Beiträge und kleine Auskünfte.
class RjPluginUi
{
public:
	virtual ~RjPluginUi() = default;
	virtual QString id() const = 0;
	/// Beim Aufbau des Hauptfensters: Menüeinträge und Klickverhalten anmelden (RjHostWindow::hooks()). Die
	/// Oberflächen werden in Ladereihenfolge angebunden (Oberfunktion vor ihren Kindern).
	virtual void attach(RjHostWindow* w) { Q_UNUSED(w) }
	/// Anschlussstelle für andere Erweiterungen: Zeiger auf das benannte Interface (z. B. einer Oberfunktion für
	/// Hersteller-Erweiterungen), sonst nullptr. Der Kern vermittelt nur (RjHostWindow::pluginUi). Der Name trägt die
	/// Fassung („name@3“); bei anderer oder fehlender Fassung liefert der Anbieter nullptr, damit nie über einen
	/// fremden Aufbau aufgerufen wird.
	virtual void* interface(const char* name) { Q_UNUSED(name) return nullptr; }
	virtual RjOverviewPart* overviewPart(RjHostOverview* tab) { Q_UNUSED(tab) return nullptr; }
	virtual RjSettingsPart* settingsPart(RjHostSettings* tab) { Q_UNUSED(tab) return nullptr; }
	virtual RjEditorPart* editorPart(RjHostProperties* tab) { Q_UNUSED(tab) return nullptr; }
	/// Anzeigename einer eigenen Zielart (Ordnerübersicht); leer = nicht meine.
	virtual QString kindName(const QString& kind) const { Q_UNUSED(kind) return {}; }
	/// Anderer Text für einen Schlüssel des Kerns (z. B. Hinweise, die mit der Erweiterung mehr abdecken); leer = Kern.
	virtual QString textFor(const char* key) const { Q_UNUSED(key) return {}; }
	/// Netzgerät mit eigener Kachelbelegung in der Übersicht?
	virtual bool isNetworkDevice(const QJsonArray& fields) const { Q_UNUSED(fields) return false; }
	/// Systemzeile eines Netzgeräts in Kopfzeile und Version teilen; `false` = nicht meine.
	virtual bool splitDeviceVersion(const QString& system, QString* head, QString* version) const { Q_UNUSED(system) Q_UNUSED(head) Q_UNUSED(version) return false; }
	/// Zusammenfassung der Sammelinventur mit eigener Zählung (`controller`); leer = Kern.
	virtual QString massSummary(bool running, int done, int total, int found, int controller, int failed, int skipped) const
	{
		Q_UNUSED(running) Q_UNUSED(done) Q_UNUSED(total) Q_UNUSED(found) Q_UNUSED(controller) Q_UNUSED(failed) Q_UNUSED(skipped)
		return {};
	}
};

/// Tab einer Aufgabe der Erweiterung im Hauptfenster: die Signale verbindet der Kern beim Öffnen (RjHostWindow::
/// addToolTab), `wipeSecrets()` ruft er beim Schließen. Schließen verwirft eingegebene Geheimnisse.
class RjPluginTab : public QWidget
{
	Q_OBJECT
public:
	using QWidget::QWidget;
	Q_INVOKABLE virtual void wipeSecrets() {}

signals:
	/// Der Index hat sich geändert (Baum neu laden).
	void indexChanged();
	void status(const QString& text);
	/// Der Tab ist erledigt und kann geschlossen werden.
	void closeRequested();
};

/// Einsprungpunkt der Bibliothek (C-Verknüpfung, exportiert).
using RjPluginUiCreate = RjPluginUi* (*)(RjHostBackend* backend, int hostUiVersion);

// --- Nur im Kern (rjplugins.cpp) ---------------------------------------------------------------------------------

class RjBackend;
/// Der Kern als RjHostBackend (Adapter, einmal je Programm).
RjHostBackend* rjHostBackend(RjBackend* backend);

/// Geladene und freigeschaltete Erweiterungen (Reihenfolge der Anmeldung).
const std::vector<RjPluginUi*>& rjPluginUis(RjBackend* backend);

/// Erster nicht leerer Text einer Erweiterung für `key`, sonst der Text des Kerns.
QString rjPluginText(RjBackend* backend, const char* key);
