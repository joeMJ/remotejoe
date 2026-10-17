#include "app.h"

#include "mainwindow.h"
#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QDir>
#include <QtCore/QElapsedTimer>
#include <QtCore/QFile>
#include <QtCore/QLibraryInfo>
#include <QtCore/QTimer>
#include <QtCore/QTranslator>
#include <QtWidgets/QApplication>
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
	w.setFilter(QString());
	w.showCredentialChoiceForScreenshot();
	pump(300);
	save(w.findChild<QWidget*>(QStringLiteral("credentialDialog")), dir, QStringLiteral("dialog-zugangsdaten.png"));
	if (auto* d = w.findChild<QWidget*>(QStringLiteral("credentialDialog")))
		d->close();
	w.patchdayForFirstTargets(25);
	pump(1500);
	save(w.findChild<QWidget*>(QStringLiteral("patchdayDialog")), dir, QStringLiteral("patchday.png"));
	if (auto* d = w.findChild<QWidget*>(QStringLiteral("patchdayDialog")))
		d->close();
	w.detachTabAt(1);
	pump(400);
	save(w.detachedWindow(), dir, QStringLiteral("abgerissenes-fenster.png"));
	QFile f(QDir(dir).filePath(QStringLiteral("messwerte.txt")));
	if (f.open(QIODevice::WriteOnly))
		f.write(QStringLiteral("erstes_zeichnen_ms=%1\nbaum_eintraege=%2\n").arg(w.firstPaintMs).arg(w.treeRowCount()).toUtf8());
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
