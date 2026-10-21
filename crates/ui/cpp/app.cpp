#include "app.h"

#include "diag.h"

#include "editor.h"
#include "mainwindow.h"
#include "titlebar.h"
#include <QtWidgets/QTabBar>
#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QCryptographicHash>
#include <QtCore/QDir>
#include <QtCore/QElapsedTimer>
#include <QtCore/QFile>
#include <QtCore/QLibraryInfo>
#include <QtCore/QLockFile>
#include <QtCore/QStandardPaths>
#include <QtCore/QThread>
#include <QtCore/QTimer>
#include <QtCore/QTranslator>
#include <QtGui/QGuiApplication>
#include <QtGui/QIcon>
#include <QtNetwork/QLocalServer>
#include <QtNetwork/QLocalSocket>
#include <QtWidgets/QApplication>
#include <QtWidgets/QAbstractItemView>
#include <QtWidgets/QComboBox>
#include <QtGui/QMouseEvent>
#include <QtCore/QPointer>
#include <QtWidgets/QLabel>
#include <QtWidgets/QMenu>
#include <QtWidgets/QPlainTextEdit>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QTabWidget>
#include <cstdio>

namespace {

void pump(int ms)
{
	QElapsedTimer t;
	t.start();
	while (t.elapsed() < ms)
		QApplication::processEvents(QEventLoop::AllEvents, 20);
}

bool save(QWidget* w, const QString& dir, const QString& name)
{
	if (!w)
		return false;
	const QString path = QDir(dir).filePath(name);
	const bool ok = w->grab().save(path);
	std::fprintf(stderr, "Bildschirmfoto %s: %s\n", qPrintable(path), ok ? "gespeichert" : "fehlgeschlagen");
	return ok;
}

/// Bildschirmfotos der Hauptansichten mit Vorführdaten, danach Ende.
void screenshots(MainWindow& w, const QString& dir)
{
	QDir().mkpath(dir);
	pump(600);
	// Web-Sitzungen (RJ_DEMO_WEB): Seite im Tab, gerendert im Worker ohne eigenes Fenster.
	if (w.showWebTabForScreenshot())
	{
		pump(6000);
		save(&w, dir, QStringLiteral("web-tab.png"));
	}
	w.tabs()->setCurrentIndex(0);
	pump(300);
	save(&w, dir, QStringLiteral("hauptfenster-hell.png"));
	// Eingeklappter Baum (F9) als Dokumentationsbild.
	w.setTreeVisible(false);
	pump(500);
	save(&w, dir, QStringLiteral("hauptfenster-kompakt-ohne-baum.png"));
	w.setTreeVisible(true);
	pump(500);
	// Dunkles Bild mit Catppuccin Macchiato (Vorgabe für Bildschirmfotos, siehe appearance.rs).
	qputenv("REMOTEJOE_THEME", "dark");
	qputenv("RJ_SHOT_DARK_THEME", "eingebaut:catppuccin-macchiato");
	w.applyTheme(true);
	pump(300);
	save(&w, dir, QStringLiteral("hauptfenster-dunkel.png"));
	qunsetenv("RJ_SHOT_DARK_THEME");
	qunsetenv("REMOTEJOE_THEME");
	w.applyTheme(false);
	w.setFilter(QStringLiteral("ssh-00"));
	pump(300);
	save(&w, dir, QStringLiteral("suche.png"));
	w.setFilter(QStringLiteral("proxmox-000"));
	w.expandFirstHypervisor();
	pump(300);
	save(&w, dir, QStringLiteral("hypervisor.png"));
	w.setFilter(QString());
	// Rückfragen als Leisten und Tabs im Hauptfenster.
	if (QWidget* p = w.showHostKeyQuestionForScreenshot(false))
	{
		pump(300);
		save(&w, dir, QStringLiteral("hostschluessel-unbekannt.png"));
		if (auto* r = p->findChild<QPushButton*>(QStringLiteral("securityReject")))
			r->click();
	}
	if (QWidget* p = w.showHostKeyQuestionForScreenshot(true))
	{
		pump(300);
		save(&w, dir, QStringLiteral("hostschluessel-geaendert.png"));
		if (auto* r = p->findChild<QPushButton*>(QStringLiteral("securityReject")))
			r->click();
	}
	w.showCredentialChoiceForScreenshot();
	pump(300);
	save(&w, dir, QStringLiteral("verbinden-zugangsdaten.png"));
	if (auto* t = w.findChild<ToolTab*>(QStringLiteral("credentialPrompt")))
	{
		w.tabs()->removeTab(w.tabs()->indexOf(t));
		t->deleteLater();
	}
	{
		// Vorführbericht (erfundene Namen) mit allen Bereichen der Vorschau.
		const QJsonObject report{
			{ QStringLiteral("folders"), 4 }, { QStringLiteral("targets"), 37 }, { QStringLiteral("credentials_in_source"), 15 },
			{ QStringLiteral("credentials_unique"), 3 }, { QStringLiteral("references"), 11 }, { QStringLiteral("encrypted_passwords"), 3 },
			{ QStringLiteral("encrypted_entries"),
			  QJsonArray{ QJsonArray{ QStringLiteral("target"), QStringLiteral("srv-01") }, QJsonArray{ QStringLiteral("target"), QStringLiteral("srv-02") },
			              QJsonArray{ QStringLiteral("credential"), QStringLiteral("Dienstkonto") } } },
			{ QStringLiteral("warnings"),
			  QJsonArray{ QStringLiteral("Ziel „srv⟦U+202E⟧gpj.exe“ enthält versteckte Zeichen (Schreibrichtung)"), QStringLiteral("Name „Drucker“ kommt in „Import/Büro“ 2× vor") } },
			{ QStringLiteral("skipped"), QJsonArray{ QJsonArray{ QStringLiteral("Fernwartung"), QStringLiteral("Verbindungstyp „TeamViewer“ wird nicht unterstützt") } } },
			{ QStringLiteral("unknown_fields"), QJsonArray{ QJsonArray{ QStringLiteral("AppVersion"), 39 }, QJsonArray{ QStringLiteral("MetaInformation"), 21 } } },
		};
		w.openImport(QStringLiteral("beispiel.rdm"), report, QString());
	}
	pump(300);
	if (auto* t = w.findChild<ToolTab*>(QStringLiteral("importTab")))
	{
		save(&w, dir, QStringLiteral("import-vorschau.png"));
		w.tabs()->removeTab(w.tabs()->indexOf(t));
		t->deleteLater();
	}
	w.mergeForFirstFolder();
	pump(300);
	if (auto* t = w.findChild<ToolTab*>(QStringLiteral("mergeTab")))
	{
		save(&w, dir, QStringLiteral("zusammenfassen.png"));
		w.tabs()->removeTab(w.tabs()->indexOf(t));
		t->deleteLater();
	}
	w.patchdayForFirstTargets(25);
	pump(1500);
	QString patchday;
	save(w.findChild<QWidget*>(QStringLiteral("patchdayTab")), dir, QStringLiteral("patchday.png"));
	// Lauf über alle 25 Ziele beobachten, dann Messwerte festhalten.
	pump(6000);
	patchday = w.patchdayMeasurement();
	if (auto* t = w.findChild<ToolTab*>(QStringLiteral("patchdayTab")))
	{
		w.tabs()->removeTab(w.tabs()->indexOf(t));
		t->deleteLater();
	}
	// Bearbeiten: Eigenschaften-Tab, Zugangsdaten-Auswahl, Hinzufügen-Menü, Entsperren.
	if (PropertiesTab* ed = w.openFirstTargetProperties())
	{
		pump(300);
		save(ed, dir, QStringLiteral("eigenschaften.png"));
		ed->selectCategory(QStringLiteral("connection"));
		pump(300);
		save(ed, dir, QStringLiteral("eigenschaften-verbindung.png"));
		ed->selectCategory(QStringLiteral("credentials"));
		if (auto* r = ed->findChild<QWidget*>(QStringLiteral("cred_named")))
			QMetaObject::invokeMethod(r, "click");
		pump(300);
		save(ed, dir, QStringLiteral("zugangsdaten-auswahl.png"));
		w.tabs()->removeTab(w.tabs()->indexOf(ed));
		ed->deleteLater();
	}
	// Web-Ziel: Anmeldefelder mit Automatik, Auswahl im Feld und „Felder erkennen“.
	if (PropertiesTab* web = w.openFirstTargetProperties(QStringLiteral("web")))
	{
		web->selectCategory(QStringLiteral("connection"));
		QJsonObject found{ { QStringLiteral("target"), web->nodeId() },
			               { QStringLiteral("fields"),
			                 QJsonObject{ { QStringLiteral("users"), QJsonArray{ QJsonObject{ { QStringLiteral("label"), QStringLiteral("Benutzername") }, { QStringLiteral("selector"), QStringLiteral("#username") } } } },
			                              { QStringLiteral("passwords"), QJsonArray{ QJsonObject{ { QStringLiteral("label"), QStringLiteral("Kennwort") }, { QStringLiteral("selector"), QStringLiteral("#password") } } } },
			                              { QStringLiteral("submits"), QJsonArray{ QJsonObject{ { QStringLiteral("label"), QStringLiteral("Anmelden") }, { QStringLiteral("selector"), QStringLiteral("#login") } } } } } } };
		web->setDetectedFields(found);
		pump(300);
		save(web, dir, QStringLiteral("web-verbindung.png"));
		w.tabs()->removeTab(w.tabs()->indexOf(web));
		web->deleteLater();
	}
	if (QMenu* m = w.popupAddMenu())
	{
		pump(300);
		save(m, dir, QStringLiteral("hinzufuegen-menue.png"));
		m->close();
	}
	QString extra;
	if (BulkAddTab* b = w.openBulkAdd(QString()))
	{
		b->hosts()->setPlainText(QStringLiteral("# Webserver Nord\nweb01.example.test\nweb02.example.test:2222\n\nweb 03\nweb04\u200B.example.test\n192.0.2.40\n"
		                                        "[2001:db8::5]:2200\nweb01.example.test"));
		b->refresh();
		pump(300);
		save(b, dir, QStringLiteral("mehrere-ziele.png"));
		if (auto* s = b->findChild<QLabel*>(QStringLiteral("bulkSummary")))
			extra += QStringLiteral("mehrere_ziele=") + s->text() + QLatin1Char('\n');
		w.tabs()->removeTab(w.tabs()->indexOf(b));
		b->deleteLater();
	}
	// Lange Tab-Titel werden gekürzt; die Symbole zeigen das Protokoll.
	if (w.tabs()->count() > 1)
	{
		const QString original = w.tabs()->tabText(1);
		w.tabs()->setTabText(1, QStringLiteral("Webseite mit sehr langem Namen – Verwaltung Server 02 im Rechenzentrum Nord"));
		pump(200);
		save(w.tabs(), dir, QStringLiteral("tabs-gekuerzt.png"));
		w.tabs()->setTabText(1, original);
	}
	w.showUnlock(false);
	pump(300);
	save(&w, dir, QStringLiteral("entsperren.png"));
	w.showNewVault();
	pump(300);
	save(&w, dir, QStringLiteral("kdbx-neu.png"));
	if (auto* o = w.findChild<QWidget*>(QStringLiteral("unlockOverlay")))
		o->hide();
	// Terminal-Einstellungen und Terminals im Schema Catppuccin Mocha.
	w.terminalSettingsForScreenshot(QStringLiteral("catppuccin-mocha"));
	pump(300);
	save(&w, dir, QStringLiteral("terminal-einstellungen.png"));
	w.tabs()->setCurrentIndex(0);
	pump(300);
	save(&w, dir, QStringLiteral("terminal-mocha.png"));
	w.macrosForScreenshot();
	pump(300);
	save(&w, dir, QStringLiteral("makro-editor.png"));
	w.tabs()->setCurrentIndex(0);
	pump(300);
	save(&w, dir, QStringLiteral("sitzung-makros.png"));
	w.showEmptyTreeForScreenshot();
	pump(300);
	save(&w, dir, QStringLiteral("leerer-baum.png"));
	w.setFilter(QString());
	// Update-Tab gegen eine lokale Attrappe (RJ_DEMO_UPDATE = Basisadresse).
	if (const QString base = qEnvironmentVariable("RJ_DEMO_UPDATE"); !base.isEmpty())
	{
		w.updateForScreenshot(base);
		pump(2500);
		save(&w, dir, QStringLiteral("update-tab.png"));
	}
	// Extras → Einstellungen (Kartenstil): Sperre und Zwischenablage, Darstellung; dann der Über-Dialog.
	w.settingsForScreenshot(QStringLiteral("lock"));
	pump(300);
	save(w.tabs()->currentWidget(), dir, QStringLiteral("einstellungen.png"));
	w.appearanceForScreenshot();
	pump(300);
	save(w.tabs()->currentWidget(), dir, QStringLiteral("darstellung.png"));
	if (QWidget* about = w.aboutForScreenshot())
	{
		pump(300);
		save(about, dir, QStringLiteral("ueber.png"));
		about->close();
	}
	w.tabs()->setCurrentIndex(0);
	// Ein Terminal-Tab als eigenes Fenster (kein Verlaufsbild: das wäre fast einfarbig und winzig als PNG).
	w.detachTabAt(w.tabs()->count() > 3 ? 3 : 1);
	pump(400);
	save(w.detachedWindow(), dir, QStringLiteral("abgerissenes-fenster.png"));
	QFile f(QDir(dir).filePath(QStringLiteral("messwerte.txt")));
	if (f.open(QIODevice::WriteOnly))
		f.write((QStringLiteral("erstes_zeichnen_ms=%1\nbaum_eintraege=%2\n").arg(w.firstPaintMs).arg(w.treeRowCount()) + patchday + extra).toUtf8());
	std::fprintf(stderr, "Erstes Zeichnen nach %lld ms, Baumeinträge: %d\n", (long long)w.firstPaintMs, w.treeRowCount());
}

// Einzelinstanz: Eine Sperrdatei im Laufzeitordner des Benutzers und ein lokaler Server.
// Ein zweiter Start meldet sich beim laufenden Programm (Fenster nach vorn) und endet.
// Je Datenordner eine Instanz: Mit REMOTEJOE_HOME (z. B. Testdaten) läuft eine eigene Instanz neben der normalen.
// Dieselbe Schlüsseldatenbank zweimal zu öffnen verhindert weiterhin deren eigene Sperre.
struct SingleInstance
{
	QLockFile lock;
	QLocalServer server;
	QString serverName;

	SingleInstance()
		: lock(baseDir() + QStringLiteral("/remotejoe%1.lock").arg(homeSuffix()))
		, serverName(QStringLiteral("remotejoe-%1%2").arg(qEnvironmentVariable("USER", qEnvironmentVariable("USERNAME", QStringLiteral("benutzer"))), homeSuffix()))
	{
		lock.setStaleLockTime(0); // veraltet nur, wenn der Prozess nicht mehr läuft
	}

	/// Leer ohne REMOTEJOE_HOME, sonst „-“ und ein Kurzhash des Ordners.
	static QString homeSuffix()
	{
		const QString home = qEnvironmentVariable("REMOTEJOE_HOME").trimmed();
		if (home.isEmpty())
			return QString();
		const QString clean = QDir::cleanPath(QDir(home).absolutePath());
		return QStringLiteral("-") + QString::fromLatin1(QCryptographicHash::hash(clean.toUtf8(), QCryptographicHash::Sha256).toHex().left(10));
	}

	static QString baseDir()
	{
		QString dir = QStandardPaths::writableLocation(QStandardPaths::RuntimeLocation);
		if (dir.isEmpty())
			dir = QStandardPaths::writableLocation(QStandardPaths::TempLocation);
		QDir().mkpath(dir);
		return dir;
	}

	// true: Wir sind die erste Instanz.
	bool acquire()
	{
		// Neustart nach einem Update: warten, bis die alte Instanz beendet ist.
		if (lock.tryLock(qEnvironmentVariableIsSet("RJ_RESTART_WAIT") ? 15000 : 0))
		{
			QLocalServer::removeServer(serverName);
			server.setSocketOptions(QLocalServer::UserAccessOption);
			server.listen(serverName);
			return true;
		}
		return false;
	}

	// Sagt der laufenden Instanz Bescheid; false, wenn sie nicht antwortet.
	bool notifyRunning()
	{
		for (int i = 0; i < 20; ++i) // bis ca. 2 s: die erste Instanz startet evtl. noch
		{
			QLocalSocket s;
			s.connectToServer(serverName);
			if (s.waitForConnected(100))
			{
				s.write("raise\n");
				s.waitForBytesWritten(500);
				s.disconnectFromServer();
				return true;
			}
			QThread::msleep(100);
		}
		return false;
	}
};

/// Auswahllisten: Qt öffnet die Liste beim Drücken der Maustaste. Kommt das Loslassen spät (Taste etwas länger
/// gehalten, Remotedesktop mit Verzögerung), wertet Qt es als Auswahl bzw. als Klick daneben und schließt die
/// Liste sofort wieder – es sieht aus, als öffne sie sich nicht. Dieses erste Loslassen über dem Feld selbst wird
/// deshalb verworfen; danach bleibt die Liste offen, bis ein Eintrag gewählt oder daneben geklickt wird.
///
/// Schließt sich eine Liste innerhalb von 150 ms ohne jede Eingabe, stimmt etwas nicht (so war es, als der
/// Fokus-Haken für Web-Tabs das Listenfenster aktivierte, FrameView::setWeb); dann hält eine Falldatei der
/// Diagnose den Zustand fest (diag.h).
class ComboReleaseGuard : public QObject
{
public:
	using QObject::QObject;

protected:
	bool eventFilter(QObject* o, QEvent* e) override
	{
		switch (e->type())
		{
		case QEvent::Show:
			if (o->inherits("QComboBoxPrivateContainer"))
			{
				m_popup = static_cast<QWidget*>(o);
				m_armed = true;
				m_userInput = false;
				m_shown.start();
			}
			break;
		case QEvent::Hide:
			if (o == m_popup)
			{
				m_armed = false;
				const qint64 ms = m_shown.isValid() ? m_shown.elapsed() : -1;
				rjdiag::log(QStringLiteral("[liste] versteckt nach %1 ms, Eingabe seither %2").arg(ms).arg(m_userInput ? 1 : 0));
				if (ms >= 0 && ms < 150 && !m_userInput)
					rjdiag::incident(QStringLiteral("Auswahlliste nach %1 ms ohne Eingabe geschlossen").arg(ms));
			}
			break;
		case QEvent::MouseButtonPress:
		case QEvent::MouseButtonDblClick:
		case QEvent::KeyPress:
			if (m_popup && m_popup->isVisible())
				m_userInput = true;
			break;
		case QEvent::MouseButtonRelease:
			if (m_armed)
			{
				m_armed = false;
				auto* combo = m_popup ? qobject_cast<QComboBox*>(m_popup->parentWidget()) : nullptr;
				const QPoint g = static_cast<QMouseEvent*>(e)->globalPosition().toPoint();
				if (combo && combo->rect().contains(combo->mapFromGlobal(g)))
				{
					rjdiag::log(QStringLiteral("[liste] erstes Loslassen über dem Feld verworfen"));
					return true;
				}
			}
			break;
		default:
			break;
		}
		return false;
	}

private:
	QPointer<QWidget> m_popup;
	bool m_armed = false;
	bool m_userInput = false;
	QElapsedTimer m_shown;
};

} // namespace

std::int32_t rjRunApplication()
{
	MainWindow::startClock.start();
	static int argc = 1;
	static char name[] = "remotejoe";
	static char* argv[] = { name, nullptr };
	QApplication app(argc, argv);
	rjdiag::init(app);
	app.installEventFilter(new ComboReleaseGuard(&app));
	QApplication::setApplicationName(QStringLiteral("remotejoe"));
	QGuiApplication::setDesktopFileName(QStringLiteral("remotejoe"));
	{
		// Programmsymbol (Logo) in allen Größen, aus der eingebetteten Ressource.
		QIcon icon;
		for (const int size : { 16, 24, 32, 48, 64, 128, 256, 512 })
			icon.addFile(QStringLiteral(":/logo/remotejoe-%1.png").arg(size), QSize(size, size));
		QApplication::setWindowIcon(icon);
	}
	// Deutsche Texte für Qt-Standarddialoge, falls installiert.
	static QTranslator qtDe;
	if (qtDe.load(QStringLiteral("qtbase_de"), QLibraryInfo::path(QLibraryInfo::TranslationsPath)))
		QApplication::installTranslator(&qtDe);
	const QString shots = qEnvironmentVariable("RJ_SCREENSHOTS");
	const bool demo = !shots.isEmpty() || qEnvironmentVariableIsSet("RJ_DEMO");
	// Vorführung und Bildschirmfotos dürfen parallel laufen; sie öffnen keine echte Datenbank.
	static SingleInstance single;
	const bool useSingle = !demo && !qEnvironmentVariableIsSet("RJ_ALLOW_MULTIPLE");
	if (useSingle && !single.acquire())
	{
		if (single.notifyRunning())
			std::fprintf(stderr, "remotejoe läuft bereits; das vorhandene Fenster wird nach vorn geholt.\n");
		else
			std::fprintf(stderr, "remotejoe läuft bereits (Sperre %s), reagiert aber nicht. Zweite Instanz wird nicht gestartet.\n", qPrintable(single.lock.fileName()));
		return 1;
	}
	RjBackend backend;
	backend.init(demo);
	MainWindow w(&backend);
	rjdiag::attachShortcut(&w);
	w.applyTheme(qEnvironmentVariable("REMOTEJOE_THEME") == QLatin1String("dark"));
	// --fenster-zuruecksetzen: gespeicherte Größe und Lage ignorieren (Standardgröße, Mitte des Bildschirms).
	if (!demo && !qEnvironmentVariableIsSet("RJ_RESET_WINDOW"))
		w.restoreWindowState();
	else if (!demo)
		w.resetWindow();
	// Nur noch die Platzsparvariante (Tabs in der Titelzeile); beim Start ist der Baum immer ausgeklappt.
	w.setTreeVisible(true);
	w.setCompact(true);
	// Standardgröße (1280 × 800) oder gespeicherte Größe nie größer als der sichtbare Bildschirmbereich.
	if (!demo)
		w.fitToScreen();
	w.show();
	if (!demo)
		QTimer::singleShot(0, &w, [&w] { w.fitToScreen(); });
	// Echter Start: zuerst die Entsperrmaske für die Schlüsseldatenbank.
	if (!demo)
	{
		w.showUnlock(false);
		w.tryKeyringUnlock();
	}
	if (useSingle)
		QObject::connect(&single.server, &QLocalServer::newConnection, &w, [&w] {
			while (QLocalSocket* c = single.server.nextPendingConnection())
			{
				c->deleteLater();
				w.setWindowState((w.windowState() & ~Qt::WindowMinimized) | Qt::WindowActive);
				w.show();
				w.raise();
				w.activateWindow();
			}
		});
	if (!shots.isEmpty())
	{
		screenshots(w, shots);
		return 0;
	}
	return app.exec();
}
