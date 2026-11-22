// Lizenz (Einstellungen → Lizenz) und der Hinweis nach dem Entsperren: Die Lizenz schaltet Erweiterungen frei;
// der Kern zeigt sie neutral (Namen der Erweiterungen aus ihrer Anmeldung).

#include "mainwindow.h"
#include "settingstab.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"

#include <QtCore/QCoreApplication>
#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtCore/QJsonObject>
#include <QtCore/QProcess>
#include <QtCore/QProcessEnvironment>
#include <QtGui/QPainter>
#include <QtSvg/QSvgRenderer>
#include <QtWidgets/QApplication>
#include <QtWidgets/QFormLayout>
#include <QtWidgets/QFrame>
#include <QtWidgets/QGridLayout>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QLabel>
#include <QtWidgets/QPlainTextEdit>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QStatusBar>
#include <QtWidgets/QVBoxLayout>
#include <cmath>
#include <QtCore/QUrl>
#include <QtCore/QTimer>
#include <QtWidgets/QMessageBox>

namespace
{
QString T(RjBackend* b, const char* key)
{
	return b->text(QString::fromLatin1(key));
}

/// Gedämpfter Hinweistext unter einem Feld.
QLabel* hint(const QString& text, QWidget* parent)
{
	auto* l = new QLabel(text, parent);
	l->setObjectName(QStringLiteral("rjMuted"));
	l->setWordWrap(true);
	return l;
}
} // namespace

// Lizenz: Urkunde mit Siegel und Liste der Erweiterungen, darunter die Update-Quelle;
// das Feld zum Einfügen einer neuen Lizenz klappt erst auf Knopfdruck auf.
namespace
{
QPixmap licSvg(const QString& svg, int px, qreal dpr)
{
	QPixmap pm(QSize(px, px) * dpr);
	pm.fill(Qt::transparent);
	QPainter p(&pm);
	QSvgRenderer r(svg.toUtf8());
	r.render(&p, QRectF(0, 0, px * dpr, px * dpr));
	pm.setDevicePixelRatio(dpr);
	return pm;
}

QString sealSvg(const QString& c, bool ok)
{
	// Rosette (gezackter Ring) mit Haken bzw. leer; gestrichelt ohne Lizenz.
	QString pts;
	for (int i = 0; i < 24; ++i)
	{
		const double a = i * 3.14159265 / 12.0, rr = (i % 2) ? 40.0 : 46.0;
		pts += QStringLiteral("%1,%2 ").arg(50 + rr * std::cos(a), 0, 'f', 1).arg(50 + rr * std::sin(a), 0, 'f', 1);
	}
	return QStringLiteral("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'>"
	                      "<polygon points='%1' fill='none' stroke='%2' stroke-width='3' %3/>"
	                      "<circle cx='50' cy='50' r='31' fill='none' stroke='%2' stroke-width='2'/>%4</svg>")
	    .arg(pts, c, ok ? QString() : QStringLiteral("stroke-dasharray='5 4'"),
	         ok ? QStringLiteral("<path d='M36 51 L46 61 L66 40' fill='none' stroke='%1' stroke-width='6' stroke-linecap='round' stroke-linejoin='round'/>").arg(c)
	            : QString());
}

QString markSvg(const QString& c, bool ok)
{
	return ok ? QStringLiteral("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24'><circle cx='12' cy='12' r='9' fill='none' stroke='%1' stroke-width='2'/>"
	                           "<path d='M8 12.5 L11 15.5 L16.5 9' fill='none' stroke='%1' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'/></svg>").arg(c)
	          : QStringLiteral("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24'><rect x='5' y='11' width='14' height='9' rx='2' fill='none' stroke='%1' stroke-width='2'/>"
	                           "<path d='M8 11 V8 a4 4 0 0 1 8 0 V11' fill='none' stroke='%1' stroke-width='2'/></svg>").arg(c);
}
} // namespace

void SettingsTab::buildLicensePage(QVBoxLayout* v, QWidget* w)
{
	// Alles Veränderliche liegt in einem eigenen Kasten, der nach Lizenzeingabe, Entfernen und Downloads neu
	// aufgebaut wird (rebuildLicensePage); die Zeilen der Erweiterungen aktualisieren sich einzeln (licenseResult).
	m_licenseBox = new QWidget(w);
	auto* bl = new QVBoxLayout(m_licenseBox);
	bl->setContentsMargins(0, 0, 0, 0);
	v->addWidget(m_licenseBox);
	fillLicensePage();
	// Einfügen der Lizenz: fest, nie neu aufgebaut (sonst verlöre ein offenes Kontextmenü sein Eingabefeld).
	auto T = [this](const char* k) { return rjText(m_backend, k); };
	const QJsonObject j = QJsonDocument::fromJson(m_backend->lizenzJson().toUtf8()).object();
	const QString st = j.value(QStringLiteral("status")).toString();
	const QJsonObject l = j.value(QStringLiteral("lizenz")).toObject();
	// Neue Lizenz einfügen: aufklappbar (ohne Lizenz gleich offen); daneben „Lizenz entfernen“.
	auto* toggle = new QPushButton(T("settings.license.paste.toggle"), w);
	auto* removeBtn = new QPushButton(T("settings.license.remove"), w);
	removeBtn->setObjectName(QStringLiteral("licenseRemove"));
	removeBtn->setVisible(st != QLatin1String("fehlt"));
	m_licenseRemoveBtn = removeBtn;
	auto* pasteBox = new QWidget(w);
	auto* pb = new QVBoxLayout(pasteBox);
	pb->setContentsMargins(0, 0, 0, 0);
	auto* editBox = new QPlainTextEdit(pasteBox);
	editBox->setPlaceholderText(QStringLiteral("-----BEGIN REMOTEJOE LIZENZ-----\n…\n-----END REMOTEJOE LIZENZ-----"));
	editBox->setMinimumHeight(120);
	pb->addWidget(editBox);
	auto* row = new QHBoxLayout;
	auto* apply = new QPushButton(T("settings.license.apply"), pasteBox);
	auto* result = new QLabel(pasteBox);
	result->setWordWrap(true);
	row->addWidget(apply);
	row->addWidget(result, 1);
	pb->addLayout(row);
	pasteBox->setVisible(l.isEmpty());
	auto* tl = new QHBoxLayout;
	tl->addWidget(toggle);
	tl->addWidget(removeBtn);
	tl->addStretch(1);
	v->addSpacing(4);
	v->addLayout(tl);
	v->addWidget(pasteBox);
	connect(toggle, &QPushButton::clicked, this, [pasteBox, editBox] {
		pasteBox->setVisible(!pasteBox->isVisible());
		if (pasteBox->isVisible())
			editBox->setFocus();
	});
	connect(removeBtn, &QPushButton::clicked, this, [this, T] {
		if (QMessageBox::question(this, T("settings.license.remove"), T("settings.license.remove.ask"), QMessageBox::Yes | QMessageBox::Cancel, QMessageBox::Cancel) != QMessageBox::Yes)
			return;
		const QJsonObject r = QJsonDocument::fromJson(this->edit({ { QStringLiteral("op"), QStringLiteral("lizenz_entfernen") } }).toUtf8()).object();
		if (r.contains(QStringLiteral("error")))
		{
			QMessageBox::warning(this, T("settings.license.remove"), r.value(QStringLiteral("error")).toString());
			return;
		}
		m_licenseRemoved = true;
		m_pluginLast.clear();
		emit status(T("settings.license.removed"));
		rebuildLicensePage();
	});
	connect(apply, &QPushButton::clicked, this, [this, editBox, result, T] {
		const QJsonObject r = QJsonDocument::fromJson(m_backend->lizenzSpeichern(editBox->toPlainText()).toUtf8()).object();
		if (r.contains(QStringLiteral("error")))
		{
			result->setText(T("settings.license.failed").arg(r.value(QStringLiteral("error")).toString()));
			return;
		}
		editBox->clear();
		const QString whoName = r.value(QStringLiteral("name")).toString();
		QString msg;
		if (r.value(QStringLiteral("abgelaufen")).toBool())
			msg = T("settings.license.saved.expired").arg(whoName, r.value(QStringLiteral("version_bis")).toString());
		else if (r.value(QStringLiteral("unbeschraenkt")).toBool())
			msg = T("settings.license.saved.unlimited").arg(whoName);
		else
			msg = T("settings.license.saved").arg(whoName, r.value(QStringLiteral("version_bis")).toString());
		// Download der Erweiterungen läuft an: Quelle merken, Fehler je Erweiterung (z. B. fehlende Oberfunktion) zeigen.
		const QJsonObject ext = r.value(QStringLiteral("erweiterungen")).toObject();
		m_pluginLast.clear();
		m_pluginSource = ext.value(QStringLiteral("quelle")).toString(m_pluginSource);
		for (const QJsonValue& e : ext.value(QStringLiteral("fehler")).toArray())
			m_pluginLast.insert(e.toObject().value(QStringLiteral("id")).toString(), QJsonObject{ { QStringLiteral("state"), QStringLiteral("error") }, { QStringLiteral("error"), e.toObject().value(QStringLiteral("error")).toString() } });
		m_licenseApplied = true;
		emit status(msg);
		rebuildLicensePage();
	});
	v->addWidget(hint(T("settings.license.hint"), w));
}

void SettingsTab::rebuildLicensePage()
{
	if (!m_licenseBox)
		return;
	// Nie unter einem offenen Menü oder Dialog neu aufbauen (deren Widgets wären sonst gelöscht): später erneut.
	if (QApplication::activePopupWidget() || QApplication::activeModalWidget())
	{
		if (!m_licenseRebuildPending)
		{
			m_licenseRebuildPending = true;
			QTimer::singleShot(400, this, [this] {
				m_licenseRebuildPending = false;
				rebuildLicensePage();
			});
		}
		return;
	}
	m_pluginState.clear();
	m_pluginRows.clear();
	m_licenseRestart = nullptr;
	const auto children = m_licenseBox->findChildren<QWidget*>(QString(), Qt::FindDirectChildrenOnly);
	for (QWidget* c : children)
	{
		m_licenseBox->layout()->removeWidget(c);
		c->deleteLater();
	}
	while (QLayoutItem* it = m_licenseBox->layout()->takeAt(0))
		delete it;
	fillLicensePage();
	refreshSummaries();
}

// Statuszeile einer Erweiterung aus dem Lizenz-JSON (installiert, geladen, wartend, laufender Download, letzter
// Downloadstand) – herstellerneutral; Namen kommen aus den Manifesten.
QString SettingsTab::pluginStateText(const QString& id, const QJsonObject& j, const QJsonObject& mod) const
{
	auto T = [this](const char* k) { return rjText(m_backend, k); };
	const bool licensed = mod.value(QStringLiteral("lizenziert")).toBool();
	QJsonObject inst;
	for (const QJsonValue& e : j.value(QStringLiteral("installiert")).toArray())
		if (e.toObject().value(QStringLiteral("id")).toString() == id)
			inst = e.toObject();
	const QString version = inst.value(QStringLiteral("version")).toString();
	for (const QJsonValue& r : j.value(QStringLiteral("laufend")).toArray())
		if (r.toString() == id)
			return T("settings.license.ext.downloading").arg(j.value(QStringLiteral("quelle")).toString(m_pluginSource));
	QJsonObject last = m_pluginLast.value(id);
	if (last.isEmpty())
		last = j.value(QStringLiteral("ergebnisse")).toObject().value(id).toObject();
	if (!last.isEmpty())
	{
		const QString st = last.value(QStringLiteral("state")).toString();
		if (st == QLatin1String("error"))
			return T("settings.license.ext.error").arg(last.value(QStringLiteral("error")).toString());
		if (st == QLatin1String("unavailable"))
			return T("settings.license.ext.unavailable");
		if (st == QLatin1String("incompatible"))
			return T("settings.license.ext.incompatible").arg(last.value(QStringLiteral("error")).toString());
		if (st == QLatin1String("installed"))
			return T("settings.license.ext.pending").arg(last.value(QStringLiteral("version")).toString());
	}
	if (inst.value(QStringLiteral("wartet")).toBool())
		return T("settings.license.ext.pending").arg(version);
	QStringList missing;
	for (const QJsonValue& m : mod.value(QStringLiteral("fehlt")).toArray())
		missing << QStringLiteral("„") + m.toString() + QStringLiteral("“");
	if (licensed && !missing.isEmpty())
		return T("settings.license.ext.missingdep").arg(missing.join(QStringLiteral(", ")));
	for (const QJsonValue& e : j.value(QStringLiteral("ladefehler")).toArray())
	{
		const QString pfad = e.toObject().value(QStringLiteral("pfad")).toString();
		if (pfad == id || pfad.contains(QStringLiteral("/") + id + QStringLiteral("/")) || pfad.contains(QStringLiteral("\\") + id + QStringLiteral("\\")))
			return T("settings.license.ext.error").arg(e.toObject().value(QStringLiteral("fehler")).toString());
	}
	if (inst.value(QStringLiteral("geladen")).toBool())
	{
		if (!licensed)
			return T("settings.license.ext.unlicensed").arg(version);
		if (last.value(QStringLiteral("state")).toString() == QLatin1String("uptodate"))
			return T("settings.license.ext.uptodate").arg(version);
		return T("settings.license.ext.loaded").arg(version);
	}
	if (!version.isEmpty())
		return licensed ? T("settings.license.ext.installed").arg(version) : version;
	if (licensed && j.value(QStringLiteral("status")).toString() == QLatin1String("gueltig"))
		return j.value(QStringLiteral("wartung_grund")).isString() ? T("settings.license.ext.nomaint") : T("settings.license.ext.notinstalled");
	return {};
}

void SettingsTab::licenseResult(const QJsonObject& r)
{
	const QString id = r.value(QStringLiteral("id")).toString();
	if (id.isEmpty() || !m_licenseBox)
		return;
	m_pluginLast.insert(id, r);
	if (r.value(QStringLiteral("state")).toString() == QLatin1String("installed") && m_licenseRestart)
		m_licenseRestart->setVisible(true);
	if (QLabel* l = m_pluginState.value(id))
	{
		const QJsonObject j = QJsonDocument::fromJson(m_backend->lizenzJson().toUtf8()).object();
		l->setText(pluginStateText(id, j, m_pluginRows.value(id)));
	}
	else if (!m_licenseRebuildPending)
	{
		m_licenseRebuildPending = true;
		QTimer::singleShot(0, this, [this] {
			m_licenseRebuildPending = false;
			rebuildLicensePage();
		});
	}
}

void SettingsTab::fillLicensePage()
{
	QWidget* w = m_licenseBox;
	auto* v = static_cast<QVBoxLayout*>(m_licenseBox->layout());
	auto T = [this](const char* k) { return rjText(m_backend, k); };
	const QJsonObject j = QJsonDocument::fromJson(m_backend->lizenzJson().toUtf8()).object();
	const QString st = j.value(QStringLiteral("status")).toString();
	const QJsonObject l = j.value(QStringLiteral("lizenz")).toObject();
	const bool dark = w->palette().color(QPalette::Window).lightness() < 128;
	const qreal dpr = w->devicePixelRatioF();
	// Gemessene Farben (WCAG ≥ 4,5:1 auf hell bzw. dunkel).
	const QString green = dark ? QStringLiteral("#33d17a") : QStringLiteral("#1a7544");
	const QString orange = dark ? QStringLiteral("#ff7800") : QStringLiteral("#a64800");
	const QString grey = w->palette().color(QPalette::PlaceholderText).name();
	const bool valid = st == QLatin1String("gueltig");
	const QString col = valid ? green : (st == QLatin1String("abgelaufen") || st == QLatin1String("ungueltig")) ? orange : grey;
	const QString wartungGrund = j.value(QStringLiteral("wartung_grund")).toString();
	if (const QString q = l.value(QStringLiteral("url")).toString(); !q.isEmpty())
		m_pluginSource = QUrl(q).host();

	// Neustart nötig: deutlich hervorgehoben, mit Knopf (wie „Jetzt neu starten“ nach einem Update).
	auto* pending = new QFrame(w);
	m_licenseRestart = pending;
	pending->setObjectName(QStringLiteral("rjLicenseRestart"));
	pending->setStyleSheet(QStringLiteral("QFrame#rjLicenseRestart { background: #e07000; border-radius: 6px; }"
	                                      "QFrame#rjLicenseRestart QLabel { color: #ffffff; font-weight: bold; }"));
	auto* pl = new QHBoxLayout(pending);
	pl->setContentsMargins(12, 8, 12, 8);
	auto* pendingText = new QLabel(T("settings.license.restart"), pending);
	pendingText->setWordWrap(true);
	auto* restart = new QPushButton(T("settings.license.restart.now"), pending);
	pl->addWidget(pendingText, 1);
	pl->addWidget(restart);
	bool anyInstalledResult = false;
	for (auto it = m_pluginLast.cbegin(); it != m_pluginLast.cend(); ++it)
		anyInstalledResult = anyInstalledResult || it.value().value(QStringLiteral("state")).toString() == QLatin1String("installed");
	// Installiert, aber in diesem Lauf nicht geladen (z. B. gerade heruntergeladen): erst der Neustart macht es aktiv.
	for (const QJsonValue& e : j.value(QStringLiteral("installiert")).toArray())
		if (const QJsonObject o = e.toObject(); valid && !o.value(QStringLiteral("version")).toString().isEmpty() && !o.value(QStringLiteral("geladen")).toBool())
			anyInstalledResult = true;
	const bool removed = j.value(QStringLiteral("entfernt")).toBool() || m_licenseRemoved;
	if (removed)
		pendingText->setText(T("settings.license.restart.removed"));
	pending->setVisible(j.value(QStringLiteral("neustart")).toBool() || j.value(QStringLiteral("erweiterung_wartet")).toBool() || anyInstalledResult || removed);
	connect(restart, &QPushButton::clicked, this, [this, pendingText, T] {
		if (QWidget* top = window(); top && !top->close())
			return;
		QProcess p;
		p.setProgram(QCoreApplication::applicationFilePath());
		auto env = QProcessEnvironment::systemEnvironment();
		env.insert(QStringLiteral("RJ_RESTART_WAIT"), QStringLiteral("1"));
		p.setProcessEnvironment(env);
		if (p.startDetached())
			qApp->quit();
		else
			pendingText->setText(T("settings.license.restart.failed"));
	});
	v->addWidget(pending);

	// Urkunde: doppelter Rahmen (außen kräftig, innen fein), Siegel oben rechts.
	auto* outer = new QFrame(w);
	outer->setObjectName(QStringLiteral("rjLicCert"));
	outer->setStyleSheet(QStringLiteral("QFrame#rjLicCert { border: 2px %2 %1; border-radius: 10px; }"
	                                    "QFrame#rjLicCertIn { border: 1px %2 %1; border-radius: 7px; }"
	                                    "QFrame#rjLicRule { border: none; border-top: 1px dashed %1; }"
	                                    "QFrame#rjLicList { border: 1px solid palette(mid); border-radius: 6px; }")
	                         .arg(col, st == QLatin1String("fehlt") ? QStringLiteral("dashed") : QStringLiteral("solid")));
	auto* ol = new QVBoxLayout(outer);
	ol->setContentsMargins(4, 4, 4, 4);
	auto* inner = new QFrame(outer);
	inner->setObjectName(QStringLiteral("rjLicCertIn"));
	ol->addWidget(inner);
	auto* il = new QVBoxLayout(inner);
	il->setContentsMargins(16, 14, 16, 14);
	il->setSpacing(8);

	auto* head = new QHBoxLayout;
	auto* titles = new QVBoxLayout;
	titles->setSpacing(2);
	auto* state = new QLabel(valid ? T("settings.license.cert.valid") : st == QLatin1String("abgelaufen") ? T("settings.license.cert.expired")
	                                 : st == QLatin1String("ungueltig") ? T("settings.license.cert.invalid") : T("settings.license.cert.missing"), inner);
	state->setStyleSheet(QStringLiteral("color: %1; font-weight: bold;").arg(col == grey ? w->palette().color(QPalette::WindowText).name() : col));
	auto* product = new QLabel(m_backend->appEdition(), inner);
	QFont pf = product->font();
	pf.setPointSizeF(pf.pointSizeF() * 1.6);
	pf.setBold(true);
	product->setFont(pf);
	QString who;
	if (!l.isEmpty())
	{
		QStringList parts;
		for (const char* k : { "unternehmen", "name", "email" })
			if (const QString s = l.value(QString::fromLatin1(k)).toString(); !s.isEmpty())
				parts << s;
		who = parts.join(QStringLiteral(" · "));
	}
	else
		who = st == QLatin1String("ungueltig") ? j.value(QStringLiteral("fehler")).toString() : T("settings.license.cert.sub.basic");
	auto* sub = new QLabel(who, inner);
	sub->setObjectName(QStringLiteral("rjMuted"));
	sub->setWordWrap(true);
	sub->setTextInteractionFlags(Qt::TextSelectableByMouse);
	titles->addWidget(state);
	titles->addWidget(product);
	titles->addWidget(sub);
	head->addLayout(titles, 1);
	auto* seal = new QLabel(inner);
	seal->setPixmap(licSvg(sealSvg(col, valid), 72, dpr));
	seal->setToolTip(valid ? T("settings.license.seal.ok") : QString());
	head->addWidget(seal, 0, Qt::AlignTop);
	il->addLayout(head);

	if (!l.isEmpty())
	{
		auto* rule = new QFrame(inner);
		rule->setObjectName(QStringLiteral("rjLicRule"));
		rule->setFixedHeight(1);
		il->addWidget(rule);
		auto* grid = new QGridLayout;
		grid->setHorizontalSpacing(24);
		auto cell = [&](int c, const QString& label, const QString& value, const QString& badge) {
			auto* lab = new QLabel(label, inner);
			lab->setObjectName(QStringLiteral("rjMuted"));
			auto* row = new QHBoxLayout;
			row->setSpacing(8);
			auto* val = new QLabel(value, inner);
			QFont f = val->font();
			f.setBold(true);
			val->setFont(f);
			row->addWidget(val);
			if (!badge.isEmpty())
			{
				auto* b = new QLabel(badge, inner);
				b->setStyleSheet(QStringLiteral("color: %1; border: 1px solid %1; border-radius: 4px; padding: 0 6px;").arg(green));
				row->addWidget(b);
			}
			row->addStretch(1);
			grid->addWidget(lab, 0, c);
			grid->addLayout(row, 1, c);
		};
		const bool unlimited = l.value(QStringLiteral("unbeschraenkt")).toBool();
		cell(0, T("settings.license.f.validity"), unlimited ? T("settings.license.unlimited") : T("settings.license.f.until").arg(l.value(QStringLiteral("version_bis")).toString()),
		     unlimited && valid ? T("settings.license.badge.permanent") : QString());
		cell(1, T("settings.license.f.maint"), T(l.value(QStringLiteral("wartung")).toBool() ? "settings.license.f.maint.on" : "settings.license.f.maint.off")
		                                           .arg(l.value(QStringLiteral("wartung_bis")).toString()), QString());
		grid->setColumnStretch(0, 1);
		grid->setColumnStretch(1, 1);
		il->addLayout(grid);
	}

	// Erweiterungen: alle bekannten (auch künftige), lizenzierte mit Haken und Stand (Version, geladen, Download),
	// nicht lizenzierte ausgegraut mit Schloss.
	auto* modTitle = new QLabel(T("settings.license.modules"), inner);
	modTitle->setObjectName(QStringLiteral("rjMuted"));
	il->addSpacing(4);
	il->addWidget(modTitle);
	auto* list = new QFrame(inner);
	list->setObjectName(QStringLiteral("rjLicList"));
	auto* ll = new QGridLayout(list);
	ll->setContentsMargins(10, 4, 10, 4);
	ll->setHorizontalSpacing(8);
	ll->setVerticalSpacing(0);
	QJsonArray mods = l.value(QStringLiteral("module")).toArray();
	if (mods.isEmpty())
		mods = j.value(QStringLiteral("alle_module")).toArray();
	int rowNo = 0;
	for (const auto& m : mods)
	{
		const QJsonObject o = m.toObject();
		const QString id = o.value(QStringLiteral("id")).toString();
		const bool on = valid && o.value(QStringLiteral("lizenziert")).toBool();
		auto* icon = new QLabel(list);
		icon->setPixmap(licSvg(markSvg(on ? green : grey, on), 18, dpr));
		icon->setContentsMargins(0, 5, 0, 5);
		auto* name = new QLabel(o.value(QStringLiteral("name")).toString(), list);
		auto* stateL = new QLabel(T(on ? "settings.license.mod.on" : "settings.license.mod.off"), list);
		auto* detail = new QLabel(pluginStateText(id, j, o), list);
		detail->setWordWrap(true);
		detail->setObjectName(QStringLiteral("rjMuted"));
		detail->setTextInteractionFlags(Qt::TextSelectableByMouse);
		if (on)
			stateL->setStyleSheet(QStringLiteral("color: %1;").arg(green));
		else
		{
			name->setObjectName(QStringLiteral("rjMuted"));
			stateL->setObjectName(QStringLiteral("rjMuted"));
			name->setEnabled(false);
			stateL->setEnabled(false);
			detail->setEnabled(false);
		}
		ll->addWidget(icon, rowNo, 0);
		ll->addWidget(name, rowNo, 1);
		ll->addWidget(detail, rowNo, 2);
		ll->addWidget(stateL, rowNo, 3, Qt::AlignRight);
		m_pluginState.insert(id, detail);
		m_pluginRows.insert(id, o);
		++rowNo;
	}
	ll->setColumnStretch(1, 1);
	ll->setColumnStretch(2, 2);
	il->addWidget(list);
	if (!l.isEmpty())
	{
		auto* foot = new QLabel(T("settings.license.sig").arg(l.value(QStringLiteral("id")).toString()), inner);
		foot->setObjectName(QStringLiteral("rjMuted"));
		foot->setTextInteractionFlags(Qt::TextSelectableByMouse);
		il->addWidget(foot);
	}
	v->addWidget(outer);

	// Update-Quelle: eigener, schlichter Kasten unter der Urkunde, mit „Erweiterungen aktualisieren“.
	if (!l.isEmpty())
	{
		auto* upd = new QFrame(w);
		upd->setObjectName(QStringLiteral("rjLicUpd"));
		upd->setStyleSheet(QStringLiteral("QFrame#rjLicUpd { border: 1px solid palette(mid); border-radius: 8px; }"));
		auto* ul = new QVBoxLayout(upd);
		ul->setContentsMargins(14, 10, 14, 10);
		auto* uh = new QHBoxLayout;
		auto* ut = new QLabel(T("settings.license.f.url"), upd);
		QFont uf = ut->font();
		uf.setBold(true);
		ut->setFont(uf);
		const bool maint = valid && l.value(QStringLiteral("wartung")).toBool();
		auto* ub = new QLabel(T(maint ? "settings.license.upd.active" : "settings.license.upd.ended"), upd);
		ub->setStyleSheet(QStringLiteral("color: %1; border: 1px solid %1; border-radius: 4px; padding: 0 6px;").arg(maint ? green : orange));
		uh->addWidget(ut, 1);
		uh->addWidget(ub);
		ul->addLayout(uh);
		auto* f = new QFormLayout;
		auto* url = new QLabel(l.value(QStringLiteral("url")).toString(), upd);
		url->setTextInteractionFlags(Qt::TextSelectableByMouse);
		url->setWordWrap(true);
		f->addRow(T("settings.license.upd.source"), url);
		f->addRow(T("settings.license.f.pw"), new QLabel(T(l.value(QStringLiteral("kennwort")).toBool() ? "settings.license.f.pw.set" : "settings.license.f.pw.none"), upd));
		ul->addLayout(f);
		auto* urow = new QHBoxLayout;
		auto* updateBtn = new QPushButton(T("settings.license.update"), upd);
		updateBtn->setObjectName(QStringLiteral("pluginsUpdate"));
		auto* updateState = new QLabel(upd);
		updateState->setWordWrap(true);
		updateState->setObjectName(QStringLiteral("rjMuted"));
		if (!maint || !valid)
		{
			updateBtn->setEnabled(false);
			updateBtn->setToolTip(wartungGrund);
			updateState->setText(wartungGrund);
		}
		urow->addWidget(updateBtn);
		urow->addWidget(updateState, 1);
		ul->addLayout(urow);
		connect(updateBtn, &QPushButton::clicked, this, [this, updateState, T] {
			const QJsonObject r = QJsonDocument::fromJson(this->edit({ { QStringLiteral("op"), QStringLiteral("plugins_update") } }).toUtf8()).object();
			if (r.contains(QStringLiteral("error")))
			{
				updateState->setText(r.value(QStringLiteral("error")).toString());
				return;
			}
			m_pluginLast.clear();
			const QJsonArray started = r.value(QStringLiteral("gestartet")).toArray();
			updateState->setText(started.isEmpty() ? T("settings.license.update.none") : T("settings.license.update.started").arg(r.value(QStringLiteral("quelle")).toString()));
			for (const QJsonValue& e : r.value(QStringLiteral("fehler")).toArray())
				m_pluginLast.insert(e.toObject().value(QStringLiteral("id")).toString(), QJsonObject{ { QStringLiteral("state"), QStringLiteral("error") }, { QStringLiteral("error"), e.toObject().value(QStringLiteral("error")).toString() } });
			rebuildLicensePage();
		});
		v->addWidget(upd);
	}

	// Der Bereich „Neue Lizenz einfügen“ steht fest unter diesem Kasten (buildLicensePage) und wird nie neu
	// aufgebaut; hier nur, ob „Lizenz entfernen“ sichtbar ist.
	if (m_licenseRemoveBtn)
		m_licenseRemoveBtn->setVisible(st != QLatin1String("fehlt") && !j.value(QStringLiteral("entfernt")).toBool());
	if (m_licenseApplied && m_licenseRestart)
		m_licenseRestart->setVisible(true);
}

QString SettingsTab::licenseSummary() const
{
	const QJsonObject j = QJsonDocument::fromJson(m_backend->lizenzJson().toUtf8()).object();
	const QString st = j.value(QStringLiteral("status")).toString();
	const QString bis = j.value(QStringLiteral("lizenz")).toObject().value(QStringLiteral("version_bis")).toString();
	if (st == QLatin1String("gueltig"))
		return j.value(QStringLiteral("lizenz")).toObject().value(QStringLiteral("unbeschraenkt")).toBool()
		           ? rjText(m_backend, "settings.license.sum.unlimited")
		           : rjText(m_backend, "settings.license.sum.valid").arg(bis);
	if (st == QLatin1String("abgelaufen"))
		return rjText(m_backend, "settings.license.sum.expired").arg(bis);
	return rjText(m_backend, "settings.license.sum.missing");
}

// Lizenz aus der Schlüsseldatenbank übernommen (gilt erst nach dem Neustart): orangefarbener Hinweis in der
// Statuszeile mit Knopf „remotejoe jetzt neu starten“ (gleicher Ablauf wie nach einem Update).
void MainWindow::licenseNoticeAfterUnlock()
{
	const QJsonObject j = QJsonDocument::fromJson(m_backend->lizenzJson().toUtf8()).object();
	if (!j.value(QStringLiteral("aus_kdbx")).toBool() || m_licenseNotice)
		return;
	m_licenseNotice = new QFrame(this);
	m_licenseNotice->setObjectName(QStringLiteral("rjLicenseRestart"));
	m_licenseNotice->setStyleSheet(QStringLiteral("QFrame#rjLicenseRestart { background: #e07000; border-radius: 4px; }"
	                                              "QFrame#rjLicenseRestart QLabel { color: #ffffff; font-weight: bold; }"));
	auto* lay = new QHBoxLayout(m_licenseNotice);
	lay->setContentsMargins(8, 2, 4, 2);
	auto* text = new QLabel(T(m_backend, "license.from_kdbx"), m_licenseNotice);
	auto* restart = new QPushButton(T(m_backend, "settings.license.restart.now"), m_licenseNotice);
	lay->addWidget(text, 1);
	lay->addWidget(restart);
	statusBar()->addPermanentWidget(m_licenseNotice, 1);
	connect(restart, &QPushButton::clicked, this, [this, text] {
		if (!close())
			return;
		QProcess p;
		p.setProgram(QCoreApplication::applicationFilePath());
		auto env = QProcessEnvironment::systemEnvironment();
		env.insert(QStringLiteral("RJ_RESTART_WAIT"), QStringLiteral("1"));
		p.setProcessEnvironment(env);
		if (p.startDetached())
			qApp->quit();
		else
			text->setText(T(m_backend, "settings.license.restart.failed"));
	});
}
