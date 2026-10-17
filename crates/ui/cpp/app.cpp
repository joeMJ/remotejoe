#include "app.h"

#include "editor.h"
#include "mainwindow.h"
#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QDir>
#include <QtCore/QElapsedTimer>
#include <QtCore/QFile>
#include <QtCore/QLibraryInfo>
#include <QtCore/QTimer>
#include <QtCore/QTranslator>
#include <QtWidgets/QApplication>
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
	save(&w, dir, QStringLiteral("hauptfenster-hell.png"));
	w.applyTheme(true);
	pump(300);
	save(&w, dir, QStringLiteral("hauptfenster-dunkel.png"));
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
	w.openImport(QStringLiteral("beispiel.rdm"), QStringLiteral("Ordner: 4\nZiele: 37 (RDP 21, SSH 12, Web 4)\nZugangsdaten: 6 (in der Gruppe „remotejoe/Import beispiel.rdm“)\n"
	                                                           "Hinweise:\n– Ziel „srv⟦U+202E⟧gpj.exe“: Schreibrichtungs-Steuerzeichen im Namen (markiert)\n– 2 Ziele ohne Host"));
	pump(300);
	if (auto* t = w.findChild<ToolTab*>(QStringLiteral("importTab")))
	{
		save(&w, dir, QStringLiteral("import-vorschau.png"));
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
		ed->selectCategory(QStringLiteral("credentials"));
		if (auto* r = ed->findChild<QWidget*>(QStringLiteral("cred_named")))
			QMetaObject::invokeMethod(r, "click");
		pump(300);
		save(ed, dir, QStringLiteral("zugangsdaten-auswahl.png"));
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
	w.showUnlock(false);
	pump(300);
	save(&w, dir, QStringLiteral("entsperren.png"));
	w.showNewVault();
	pump(300);
	save(&w, dir, QStringLiteral("kdbx-neu.png"));
	if (auto* o = w.findChild<QWidget*>(QStringLiteral("unlockOverlay")))
		o->hide();
	w.showEmptyTreeForScreenshot();
	pump(300);
	save(&w, dir, QStringLiteral("leerer-baum.png"));
	w.setFilter(QString());
	w.tabs()->setCurrentIndex(0);
	w.detachTabAt(1);
	pump(400);
	save(w.detachedWindow(), dir, QStringLiteral("abgerissenes-fenster.png"));
	QFile f(QDir(dir).filePath(QStringLiteral("messwerte.txt")));
	if (f.open(QIODevice::WriteOnly))
		f.write((QStringLiteral("erstes_zeichnen_ms=%1\nbaum_eintraege=%2\n").arg(w.firstPaintMs).arg(w.treeRowCount()) + patchday + extra).toUtf8());
	std::fprintf(stderr, "Erstes Zeichnen nach %lld ms, Baumeinträge: %d\n", (long long)w.firstPaintMs, w.treeRowCount());
}

} // namespace

std::int32_t rjRunApplication()
{
	MainWindow::startClock.start();
	static int argc = 1;
	static char name[] = "remotejoe";
	static char* argv[] = { name, nullptr };
	QApplication app(argc, argv);
	QApplication::setApplicationName(QStringLiteral("remotejoe"));
	// Deutsche Texte für Qt-Standarddialoge, falls installiert.
	static QTranslator qtDe;
	if (qtDe.load(QStringLiteral("qtbase_de"), QLibraryInfo::path(QLibraryInfo::TranslationsPath)))
		QApplication::installTranslator(&qtDe);
	const QString shots = qEnvironmentVariable("RJ_SCREENSHOTS");
	const bool demo = !shots.isEmpty() || qEnvironmentVariableIsSet("RJ_DEMO");
	RjBackend backend;
	backend.init(demo);
	MainWindow w(&backend);
	w.applyTheme(qEnvironmentVariable("REMOTEJOE_THEME") == QLatin1String("dark"));
	w.show();
	if (!shots.isEmpty())
	{
		screenshots(w, shots);
		return 0;
	}
	return app.exec();
}
