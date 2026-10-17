#include "mainwindow.h"

#include "remotejoe-ui/src/bridge.cxxqt.h"
#include "views.h"

#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtGui/QAction>
#include <QtGui/QClipboard>
#include <QtGui/QCloseEvent>
#include <QtGui/QFontDatabase>
#include <QtGui/QGuiApplication>
#include <QtGui/QKeySequence>
#include <QtGui/QPainter>
#include <QtGui/QShortcut>
#include <QtSvg/QSvgRenderer>
#include <QtWidgets/QApplication>
#include <QtWidgets/QCheckBox>
#include <QtWidgets/QComboBox>
#include <QtWidgets/QDialog>
#include <QtWidgets/QDialogButtonBox>
#include <QtWidgets/QFileDialog>
#include <QtWidgets/QFormLayout>
#include <QtWidgets/QHeaderView>
#include <QtWidgets/QInputDialog>
#include <QtWidgets/QLabel>
#include <QtWidgets/QMenu>
#include <QtWidgets/QMenuBar>
#include <QtWidgets/QMessageBox>
#include <QtWidgets/QSplitter>
#include <QtWidgets/QStatusBar>
#include <QtWidgets/QStyleFactory>
#include <QtWidgets/QTabBar>
#include <QtWidgets/QPlainTextEdit>
#include <QtWidgets/QProgressBar>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QTableWidget>
#include <QtWidgets/QVBoxLayout>

QElapsedTimer MainWindow::startClock;

namespace {

constexpr int RoleId = Qt::UserRole + 1;
constexpr int RoleFolder = Qt::UserRole + 2;

QString T(RjBackend* b, const char* key)
{
	return b->text(QString::fromLatin1(key));
}

QJsonObject parseObject(const QString& s)
{
	return QJsonDocument::fromJson(s.toUtf8()).object();
}

/// Symbol je Protokoll (SVG mit Kürzel), optional mit Ampelpunkt.
QIcon protocolIcon(const QString& protocol, const QString& light, bool folder)
{
	static QHash<QString, QIcon> cache;
	const QString key = protocol + QLatin1Char('|') + light + (folder ? QStringLiteral("|f") : QString());
	auto it = cache.find(key);
	if (it != cache.end())
		return *it;
	struct P
	{
		const char* name;
		const char* label;
		const char* color;
	};
	static const P table[] = { { "rdp", "R", "#0b5cad" },     { "ssh", "S", "#1b6e20" },   { "vnc", "V", "#7a3e9d" },
		                       { "web", "W", "#8a5300" },     { "psrp", "P", "#005f73" },  { "proxmox", "X", "#b03a00" },
		                       { "hyperv", "H", "#3a4a8c" } };
	QString svg;
	if (folder)
		svg = QStringLiteral("<svg xmlns='http://www.w3.org/2000/svg' width='32' height='32'>"
		                     "<path d='M2 8h11l3 3h14v17H2z' fill='#c9a227' stroke='#6b5410' stroke-width='1.5'/></svg>");
	else
	{
		QString label = QStringLiteral("?"), color = QStringLiteral("#555555");
		for (const P& p : table)
			if (protocol == QLatin1String(p.name))
			{
				label = QLatin1String(p.label);
				color = QLatin1String(p.color);
			}
		svg = QStringLiteral("<svg xmlns='http://www.w3.org/2000/svg' width='32' height='32'>"
		                     "<rect x='1' y='3' width='30' height='26' rx='5' fill='%1'/>"
		                     "<text x='16' y='23' font-size='18' font-family='sans-serif' font-weight='bold' "
		                     "text-anchor='middle' fill='#ffffff'>%2</text></svg>")
		        .arg(color, label);
	}
	QPixmap pm(32, 32);
	pm.fill(Qt::transparent);
	{
		QPainter p(&pm);
		QSvgRenderer r(svg.toUtf8());
		r.render(&p);
		if (!light.isEmpty())
		{
			QColor c = light == QLatin1String("green")    ? QColor("#1b9e20")
			           : light == QLatin1String("yellow") ? QColor("#e0a800")
			           : light == QLatin1String("red")    ? QColor("#d32f2f")
			                                              : QColor("#8a8a8a");
			p.setRenderHint(QPainter::Antialiasing);
			p.setBrush(c);
			p.setPen(QPen(Qt::white, 2));
			p.drawEllipse(QRect(19, 19, 12, 12));
		}
	}
	QIcon icon(pm);
	cache.insert(key, icon);
	return icon;
}

void addNodes(QStandardItem* parent, const QJsonArray& nodes)
{
	for (const QJsonValue& v : nodes)
	{
		const QJsonObject n = v.toObject();
		const bool folder = n.value(QStringLiteral("folder")).toBool();
		auto* item = new QStandardItem(n.value(QStringLiteral("name")).toString());
		item->setEditable(false);
		item->setData(n.value(QStringLiteral("id")).toString(), RoleId);
		item->setData(folder, RoleFolder);
		item->setIcon(protocolIcon(n.value(QStringLiteral("protocol")).toString(), n.value(QStringLiteral("light")).toString(), folder));
		QString tip = n.value(QStringLiteral("host")).toString();
		if (n.value(QStringLiteral("suspicious")).toBool())
			tip += QStringLiteral("\nName enthält unsichtbare oder richtungsändernde Zeichen.");
		item->setToolTip(tip);
		parent->appendRow(item);
		addNodes(item, n.value(QStringLiteral("children")).toArray());
	}
}

/// Dialog zur Auswahl eines Zugangsdatensatzes.
class CredentialDialog : public QDialog
{
public:
	CredentialDialog(RjBackend* b, const QJsonArray& choices, QWidget* parent)
	  : QDialog(parent)
	{
		setWindowTitle(T(b, "dialog.credential.title"));
		auto* lay = new QVBoxLayout(this);
		lay->addWidget(new QLabel(T(b, "dialog.credential.text"), this));
		combo = new QComboBox(this);
		combo->setMinimumHeight(32);
		for (const QJsonValue& c : choices)
		{
			const QJsonObject o = c.toObject();
			QString label = o.value(QStringLiteral("name")).toString();
			const QString title = o.value(QStringLiteral("title")).toString();
			if (!title.isEmpty())
				label += QStringLiteral(" (") + title + QLatin1Char(')');
			combo->addItem(label, o.value(QStringLiteral("name")).toString());
		}
		lay->addWidget(combo);
		remember = new QCheckBox(T(b, "dialog.credential.remember"), this);
		lay->addWidget(remember);
		auto* buttons = new QDialogButtonBox(QDialogButtonBox::Ok | QDialogButtonBox::Cancel, this);
		buttons->button(QDialogButtonBox::Ok)->setText(QStringLiteral("Verbinden"));
		buttons->button(QDialogButtonBox::Cancel)->setText(QStringLiteral("Abbrechen"));
		connect(buttons, &QDialogButtonBox::accepted, this, &QDialog::accept);
		connect(buttons, &QDialogButtonBox::rejected, this, &QDialog::reject);
		lay->addWidget(buttons);
	}
	QComboBox* combo;
	QCheckBox* remember;
};

/// Patchday: Zustand je Ziel, Fortschritt, Protokoll, Abbruch.
class PatchdayDialog : public QDialog
{
public:
	PatchdayDialog(RjBackend* b, const QStringList& ids, QWidget* parent)
	  : QDialog(parent)
	  , m_backend(b)
	  , m_ids(ids)
	{
		setWindowTitle(QStringLiteral("Patchday"));
		resize(900, 600);
		auto* lay = new QVBoxLayout(this);
		m_table = new QTableWidget(0, 4, this);
		m_table->setHorizontalHeaderLabels({ QStringLiteral("Ziel"), QStringLiteral("Zustand"), QStringLiteral("Anzahl"), QStringLiteral("Fortschritt") });
		m_table->horizontalHeader()->setSectionResizeMode(0, QHeaderView::Stretch);
		m_table->setSelectionBehavior(QAbstractItemView::SelectRows);
		m_table->setEditTriggers(QAbstractItemView::NoEditTriggers);
		m_table->verticalHeader()->setDefaultSectionSize(30);
		m_log = new QPlainTextEdit(this);
		m_log->setReadOnly(true);
		m_log->setMaximumHeight(160);
		auto* buttons = new QDialogButtonBox(this);
		auto* run = buttons->addButton(QStringLiteral("Ausführen …"), QDialogButtonBox::ActionRole);
		auto* cancel = buttons->addButton(QStringLiteral("Ziel abbrechen"), QDialogButtonBox::ActionRole);
		buttons->addButton(QDialogButtonBox::Close)->setText(QStringLiteral("Schließen"));
		lay->addWidget(new QLabel(QStringLiteral("Trockenlauf: Aktualisierungen werden nur aufgelistet. Ausführen erst auf Befehl."), this));
		lay->addWidget(m_table, 1);
		lay->addWidget(m_log);
		lay->addWidget(buttons);
		connect(buttons, &QDialogButtonBox::rejected, this, &QDialog::reject);
		connect(run, &QPushButton::clicked, this, [this] {
			if (QMessageBox::question(this, windowTitle(), QStringLiteral("Aktualisierungen auf %1 Zielen jetzt installieren?").arg(m_ids.size())) == QMessageBox::Yes)
				m_backend->startPatchday(QString::fromUtf8(QJsonDocument(QJsonArray::fromStringList(m_ids)).toJson(QJsonDocument::Compact)), true);
		});
		connect(cancel, &QPushButton::clicked, this, [this] {
			const int r = m_table->currentRow();
			if (r >= 0)
				m_backend->cancelPatchTarget(m_table->item(r, 0)->data(Qt::UserRole).toString());
		});
		connect(m_table, &QTableWidget::currentCellChanged, this, [this] { refresh(); });
		connect(&m_timer, &QTimer::timeout, this, [this] { refresh(); });
		m_timer.start(400);
		refresh();
	}

	void refresh()
	{
		const QJsonArray rows = QJsonDocument::fromJson(m_backend->patchdayJson().toUtf8()).array();
		m_table->setRowCount(rows.size());
		for (int i = 0; i < rows.size(); ++i)
		{
			const QJsonObject r = rows[i].toObject();
			auto set = [&](int col, const QString& text) {
				auto* it = m_table->item(i, col);
				if (!it)
				{
					it = new QTableWidgetItem;
					m_table->setItem(i, col, it);
				}
				it->setText(text);
				return it;
			};
			set(0, r.value(QStringLiteral("name")).toString())->setData(Qt::UserRole, r.value(QStringLiteral("target")).toString());
			QString state = r.value(QStringLiteral("label")).toString();
			if (r.contains(QStringLiteral("detail")))
				state += QStringLiteral(": ") + r.value(QStringLiteral("detail")).toString();
			set(1, state);
			set(2, r.value(QStringLiteral("count")).isNull() ? QString() : QString::number(r.value(QStringLiteral("count")).toInt()));
			auto* bar = qobject_cast<QProgressBar*>(m_table->cellWidget(i, 3));
			if (!bar)
			{
				bar = new QProgressBar(m_table);
				m_table->setCellWidget(i, 3, bar);
			}
			bar->setValue(r.value(QStringLiteral("percent")).toInt());
			if (i == m_table->currentRow())
			{
				QStringList log;
				for (const QJsonValue& l : r.value(QStringLiteral("log")).toArray())
					log << l.toString();
				if (m_log->toPlainText() != log.join(QLatin1Char('\n')))
					m_log->setPlainText(log.join(QLatin1Char('\n')));
			}
		}
	}

private:
	RjBackend* m_backend;
	QStringList m_ids;
	QTableWidget* m_table;
	QPlainTextEdit* m_log;
	QTimer m_timer;
};

/// Ziel-IDs der Auswahl (Ordner schließen ihre Ziele ein).
QStringList selectedTargets(QTreeView* tree, QStandardItemModel* model)
{
	QStringList ids;
	std::function<void(const QModelIndex&)> collect = [&](const QModelIndex& i) {
		if (!i.data(RoleFolder).toBool())
		{
			const QString id = i.data(RoleId).toString();
			if (!ids.contains(id))
				ids << id;
			return;
		}
		for (int r = 0; r < model->rowCount(i); ++r)
			collect(model->index(r, 0, i));
	};
	for (const QModelIndex& i : tree->selectionModel()->selectedIndexes())
		collect(i);
	return ids;
}

} // namespace

void MainWindow::openPatchday(const QStringList& ids)
{
	const QString res = m_backend->startPatchday(QString::fromUtf8(QJsonDocument(QJsonArray::fromStringList(ids)).toJson(QJsonDocument::Compact)), false);
	statusBar()->showMessage(res, 5000);
	auto* d = new PatchdayDialog(m_backend, ids, this);
	d->setAttribute(Qt::WA_DeleteOnClose);
	d->setObjectName(QStringLiteral("patchdayDialog"));
	d->show();
}

void MainWindow::patchdayForFirstTargets(int n)
{
	QStringList ids;
	std::function<void(const QModelIndex&)> walk = [&](const QModelIndex& parent) {
		for (int r = 0; r < m_model->rowCount(parent) && ids.size() < n; ++r)
		{
			const QModelIndex i = m_model->index(r, 0, parent);
			if (i.data(RoleFolder).toBool())
				walk(i);
			else
				ids << i.data(RoleId).toString();
		}
	};
	walk(QModelIndex());
	openPatchday(ids);
}

MainWindow::MainWindow(RjBackend* backend, QWidget* parent)
  : QMainWindow(parent)
  , m_backend(backend)
{
	setWindowTitle(T(m_backend, "app.title"));
	resize(1280, 800);

	auto* left = new QWidget(this);
	auto* leftLay = new QVBoxLayout(left);
	leftLay->setContentsMargins(4, 4, 4, 4);
	m_search = new QLineEdit(left);
	m_search->setPlaceholderText(T(m_backend, "search.placeholder"));
	m_search->setClearButtonEnabled(true);
	m_search->setMinimumHeight(32);
	m_search->setAccessibleName(T(m_backend, "search.placeholder"));
	m_tree = new QTreeView(left);
	m_model = new QStandardItemModel(this);
	m_tree->setModel(m_model);
	m_tree->setHeaderHidden(true);
	m_tree->setUniformRowHeights(true);
	m_tree->setIconSize(QSize(22, 22));
	m_tree->setContextMenuPolicy(Qt::CustomContextMenu);
	m_tree->setSelectionMode(QAbstractItemView::ExtendedSelection);
	m_tree->setAccessibleName(QStringLiteral("Verbindungsbaum"));
	leftLay->addWidget(m_search);
	leftLay->addWidget(m_tree, 1);

	m_tabs = new QTabWidget(this);
	m_tabs->setTabsClosable(true);
	m_tabs->setMovable(true);
	m_tabs->setDocumentMode(true);
	m_tabs->tabBar()->setContextMenuPolicy(Qt::CustomContextMenu);

	auto* split = new QSplitter(Qt::Horizontal, this);
	split->addWidget(left);
	split->addWidget(m_tabs);
	split->setStretchFactor(1, 1);
	split->setSizes({ 300, 980 });
	setCentralWidget(split);

	buildMenus();
	statusBar()->showMessage(T(m_backend, "status.ready"));

	connect(m_tree, &QTreeView::activated, this, [this](const QModelIndex& i) {
		if (!i.data(RoleFolder).toBool())
			connectSelected();
	});
	connect(m_tree, &QTreeView::customContextMenuRequested, this, &MainWindow::treeContextMenu);
	m_filterTimer.setSingleShot(true);
	m_filterTimer.setInterval(150);
	connect(m_search, &QLineEdit::textChanged, this, [this] { m_filterTimer.start(); });
	connect(&m_filterTimer, &QTimer::timeout, this, &MainWindow::reloadTree);
	connect(m_tabs, &QTabWidget::tabCloseRequested, this, [this](int i) {
		const auto tab = m_tabs->widget(i)->property("tab").toULongLong();
		for (const QJsonValue& t : m_workspace.value(QStringLiteral("tabs")).toArray())
			if (std::uint64_t(t.toObject().value(QStringLiteral("id")).toInteger()) == tab)
				for (const QJsonValue& s : t.toObject().value(QStringLiteral("sessions")).toArray())
				{
					const auto sid = std::uint64_t(s.toInteger());
					m_backend->closeSession(sid);
					if (auto* p = m_panes.take(sid))
						p->deleteLater();
				}
		rebuildTabs();
	});
	connect(m_tabs, &QTabWidget::currentChanged, this, [this](int) {
		if (auto t = currentTab())
			m_backend->activateTab(t);
		updateStatus();
	});
	connect(m_tabs->tabBar(), &QTabBar::customContextMenuRequested, this,
	        [this](const QPoint& p) { tabContextMenu(m_tabs, p); });

	// Tastaturbedienung.
	auto sc = [this](const QKeySequence& k, auto fn) {
		auto* s = new QShortcut(k, this);
		s->setContext(Qt::ApplicationShortcut);
		connect(s, &QShortcut::activated, this, fn);
	};
	sc(QKeySequence(QStringLiteral("Ctrl+F")), [this] {
		m_search->setFocus();
		m_search->selectAll();
	});
	sc(QKeySequence(QStringLiteral("Ctrl+Alt+Home")), [this] { m_tree->setFocus(); });
	sc(QKeySequence(QStringLiteral("Ctrl+PgDown")), [this] { m_tabs->setCurrentIndex((m_tabs->currentIndex() + 1) % qMax(1, m_tabs->count())); });
	sc(QKeySequence(QStringLiteral("Ctrl+PgUp")), [this] { m_tabs->setCurrentIndex((m_tabs->currentIndex() + m_tabs->count() - 1) % qMax(1, m_tabs->count())); });
	connect(m_search, &QLineEdit::returnPressed, this, [this] { m_tree->setFocus(); m_tree->setCurrentIndex(m_model->index(0, 0)); });

	m_poll.setInterval(16);
	connect(&m_poll, &QTimer::timeout, this, &MainWindow::poll);
	m_poll.start();
	reloadTree();
	rebuildTabs();
}

MainWindow::~MainWindow() = default;

void MainWindow::buildMenus()
{
	QMenu* file = menuBar()->addMenu(T(m_backend, "menu.file"));
	file->addAction(T(m_backend, "menu.file.open"), QKeySequence::Open, this, [this] { unlockDialog(); });
	file->addAction(T(m_backend, "menu.file.save"), QKeySequence::Save, this, [this] { statusBar()->showMessage(m_backend->save(), 5000); });
	file->addAction(T(m_backend, "menu.file.lock"), QKeySequence(QStringLiteral("Ctrl+L")), this, [this] {
		m_backend->lock();
		reloadTree();
		statusBar()->showMessage(T(m_backend, "status.locked"));
	});
	file->addSeparator();
	file->addAction(T(m_backend, "menu.file.quit"), QKeySequence::Quit, this, &QWidget::close);
	QMenu* view = menuBar()->addMenu(T(m_backend, "menu.view"));
	const char* modes[] = { "single", "one_by_two", "two_by_two", "free" };
	int n = 1;
	for (const char* m : modes)
	{
		const QString mode = QString::fromLatin1(m);
		view->addAction(m_backend->text(QStringLiteral("menu.view.") + mode), QKeySequence(QStringLiteral("Alt+%1").arg(n++)), this, [this, mode] {
			if (m_backend->setArrangement(currentTab(), mode))
				rebuildTabs();
		});
	}
	view->addSeparator();
	view->addAction(T(m_backend, "menu.view.detach"), QKeySequence(QStringLiteral("Ctrl+Shift+D")), this, [this] {
		if (m_backend->detachTab(currentTab()))
			rebuildTabs();
	});
}

void MainWindow::applyTheme(bool dark)
{
	qApp->setStyle(QStyleFactory::create(QStringLiteral("Fusion")));
	QJsonObject p = parseObject(m_backend->paletteJson());
	if (dark != (QColor(p.value(QStringLiteral("base")).toString()).lightness() < 128))
	{
		// Gegenteiliges Schema erzwingen (für Bildschirmfotos).
		qputenv("REMOTEJOE_THEME", dark ? "dark" : "light");
		p = parseObject(m_backend->paletteJson());
	}
	auto c = [&](const char* k) { return QColor(p.value(QString::fromLatin1(k)).toString()); };
	QPalette pal;
	pal.setColor(QPalette::Window, c("window"));
	pal.setColor(QPalette::WindowText, c("text"));
	pal.setColor(QPalette::Base, c("base"));
	pal.setColor(QPalette::AlternateBase, c("alternate"));
	pal.setColor(QPalette::Text, c("text"));
	pal.setColor(QPalette::PlaceholderText, c("muted"));
	pal.setColor(QPalette::Button, c("button"));
	pal.setColor(QPalette::ButtonText, c("text"));
	pal.setColor(QPalette::Highlight, c("highlight"));
	pal.setColor(QPalette::HighlightedText, c("highlight_text"));
	pal.setColor(QPalette::Link, c("link"));
	pal.setColor(QPalette::ToolTipBase, c("base"));
	pal.setColor(QPalette::ToolTipText, c("text"));
	qApp->setPalette(pal);
	qApp->setStyleSheet(QStringLiteral(
	  "QTreeView::item { min-height: 28px; padding: 2px; }"
	  "QTabBar::tab { min-height: 30px; padding: 4px 12px; }"
	  "QLabel#paneHeader { font-weight: bold; padding: 4px; }"
	  "QLabel#paneNotice { color: %1; padding: 4px; }"
	  "QPushButton { min-height: 30px; padding: 2px 12px; }")
	                      .arg(c("warning").name()));
}

void MainWindow::reloadTree()
{
	const QString json = m_backend->treeJson(m_search->text());
	m_model->clear();
	addNodes(m_model->invisibleRootItem(), QJsonDocument::fromJson(json.toUtf8()).array());
	if (!m_search->text().isEmpty())
		m_tree->expandAll();
	else
		m_tree->expandToDepth(0);
}

int MainWindow::treeRowCount() const
{
	int n = 0;
	std::function<void(const QModelIndex&)> walk = [&](const QModelIndex& parent) {
		for (int r = 0; r < m_model->rowCount(parent); ++r)
		{
			++n;
			walk(m_model->index(r, 0, parent));
		}
	};
	walk(QModelIndex());
	return n;
}

void MainWindow::setFilter(const QString& text)
{
	m_search->setText(text);
	reloadTree();
}

QSize MainWindow::contentSize(bool terminal) const
{
	QSize s = m_tabs->size() - QSize(8, 40);
	if (!terminal)
		return s;
	QFontMetrics fm(QFontDatabase::systemFont(QFontDatabase::FixedFont));
	return QSize(qMax(20, s.width() / qMax(1, fm.horizontalAdvance(QLatin1Char('M')))), qMax(5, s.height() / qMax(1, fm.height())));
}

void MainWindow::connectSelected(const QString& credential, bool remember)
{
	const QModelIndex i = m_tree->currentIndex();
	if (!i.isValid() || i.data(RoleFolder).toBool())
		return;
	const QString id = i.data(RoleId).toString();
	const bool term = m_tree->currentIndex().data(Qt::ToolTipRole).isValid();
	Q_UNUSED(term);
	const QSize px = contentSize(false);
	const QString res = m_backend->connectTarget(id, credential, remember, px.width(), px.height());
	handleConnectResult(id, res);
}

void MainWindow::handleConnectResult(const QString& id, const QString& json)
{
	const QJsonObject r = parseObject(json);
	if (r.contains(QStringLiteral("session")))
	{
		rebuildTabs();
		const auto s = std::uint64_t(r.value(QStringLiteral("session")).toInteger());
		if (auto* p = pane(s))
		{
			if (p->terminal())
			{
				const QSize c = contentSize(true);
				m_backend->resizeSession(s, c.width(), c.height());
			}
			p->view()->setFocus();
		}
		return;
	}
	if (r.contains(QStringLiteral("ambiguous")))
	{
		CredentialDialog d(m_backend, r.value(QStringLiteral("ambiguous")).toArray(), this);
		if (d.exec() == QDialog::Accepted)
		{
			const QSize px = contentSize(false);
			handleConnectResult(id, m_backend->connectTarget(id, d.combo->currentData().toString(), d.remember->isChecked(), px.width(), px.height()));
		}
		return;
	}
	if (r.contains(QStringLiteral("hostkey")))
	{
		// Unbekannter Host-Schlüssel eines Sprungservers: nie still annehmen.
		const QJsonObject h = r.value(QStringLiteral("hostkey")).toObject();
		QMessageBox box(QMessageBox::Warning, T(m_backend, "app.title"), h.value(QStringLiteral("text")).toString(), QMessageBox::NoButton, this);
		box.setInformativeText(h.value(QStringLiteral("detail")).toString());
		QPushButton* yes = box.addButton(T(m_backend, "dialog.question.accept"), QMessageBox::AcceptRole);
		QPushButton* no = box.addButton(T(m_backend, "dialog.question.reject"), QMessageBox::RejectRole);
		box.setDefaultButton(no);
		box.exec();
		if (box.clickedButton() == yes &&
		    m_backend->acceptHostKey(h.value(QStringLiteral("host")).toString(), std::uint32_t(h.value(QStringLiteral("port")).toInt()),
		                             h.value(QStringLiteral("key")).toString()))
		{
			const QSize px = contentSize(false);
			handleConnectResult(id, m_backend->connectTarget(id, QString(), false, px.width(), px.height()));
		}
		return;
	}
	if (r.contains(QStringLiteral("missing")))
	{
		const QJsonObject m = r.value(QStringLiteral("missing")).toObject();
		QString title = m.value(QStringLiteral("title")).toString();
		QMessageBox::warning(this, T(m_backend, "missing.credential"),
		                     QStringLiteral("%1: „%2“\n%3\n\nÜber „Nicht zugeordnet“ kann ein vorhandener Eintrag neu zugeordnet werden.")
		                       .arg(T(m_backend, "missing.credential"), title.isEmpty() ? m.value(QStringLiteral("name")).toString() : title,
		                            m.value(QStringLiteral("reason")).toString()));
		return;
	}
	QMessageBox::warning(this, T(m_backend, "app.title"), r.value(QStringLiteral("error")).toString());
}

void MainWindow::showProperties(const QString& id)
{
	const QJsonObject p = parseObject(m_backend->propertiesJson(id));
	QDialog d(this);
	d.setWindowTitle(T(m_backend, "dialog.properties.title"));
	auto* form = new QFormLayout(&d);
	const char* keys[][2] = { { "path", "Pfad" },   { "protocol", "Protokoll" },          { "host", "Host" },
		                      { "port", "Port" },   { "credential", "Zugangsdatensatz" }, { "credential_entry", "KDBX-Eintrag" },
		                      { "tunnel", "Tunnel" } };
	for (auto& k : keys)
	{
		const QJsonValue v = p.value(QString::fromLatin1(k[0]));
		if (v.isUndefined() || v.isNull())
			continue;
		auto* l = new QLabel(v.isDouble() ? QString::number(v.toInt()) : v.toString(), &d);
		l->setTextInteractionFlags(Qt::TextSelectableByMouse | Qt::TextSelectableByKeyboard);
		form->addRow(QString::fromUtf8(k[1]) + QLatin1Char(':'), l);
	}
	auto* b = new QDialogButtonBox(QDialogButtonBox::Close, &d);
	b->button(QDialogButtonBox::Close)->setText(QStringLiteral("Schließen"));
	connect(b, &QDialogButtonBox::rejected, &d, &QDialog::reject);
	form->addRow(b);
	d.exec();
}

void MainWindow::treeContextMenu(const QPoint& pos)
{
	const QModelIndex i = m_tree->indexAt(pos);
	if (!i.isValid())
		return;
	if (!m_tree->selectionModel()->isSelected(i))
		m_tree->setCurrentIndex(i);
	const QString id = i.data(RoleId).toString();
	QMenu menu(this);
	if (!i.data(RoleFolder).toBool())
	{
		menu.addAction(T(m_backend, "ctx.connect"), this, [this] { connectSelected(); });
		menu.addAction(T(m_backend, "ctx.tile"), this, [this] {
			const auto tab = currentTab();
			connectSelected();
			const auto s = currentSession();
			if (tab && s && m_backend->tileSession(s, tab))
				rebuildTabs();
		});
	}
	QMenu* tasks = menu.addMenu(T(m_backend, "ctx.tasks"));
	tasks->addAction(T(m_backend, "ctx.tasks.updates"), this, [this] {
		const QStringList ids = selectedTargets(m_tree, m_model);
		if (!ids.isEmpty())
			openPatchday(ids);
	});
	menu.addAction(T(m_backend, "ctx.properties"), this, [this, id] { showProperties(id); });
	menu.exec(m_tree->viewport()->mapToGlobal(pos));
}

void MainWindow::tabContextMenu(QTabWidget* tabs, const QPoint& pos)
{
	const int idx = tabs->tabBar()->tabAt(pos);
	if (idx < 0)
		return;
	const auto tab = tabs->widget(idx)->property("tab").toULongLong();
	QMenu menu(this);
	if (tabs == m_tabs)
		menu.addAction(T(m_backend, "menu.view.detach"), this, [this, tab] {
			if (m_backend->detachTab(tab))
				rebuildTabs();
		});
	else
		menu.addAction(QStringLiteral("Zurück ins Hauptfenster"), this, [this, tab] {
			if (m_backend->attachTab(tab))
				rebuildTabs();
		});
	const char* modes[] = { "single", "one_by_two", "two_by_two", "free" };
	for (const char* m : modes)
	{
		const QString mode = QString::fromLatin1(m);
		menu.addAction(m_backend->text(QStringLiteral("menu.view.") + mode), this, [this, tab, mode] {
			if (m_backend->setArrangement(tab, mode))
				rebuildTabs();
		});
	}
	menu.exec(tabs->tabBar()->mapToGlobal(pos));
}

void MainWindow::unlockDialog()
{
	QString path = m_backend->kdbxPath();
	path = QFileDialog::getOpenFileName(this, T(m_backend, "dialog.unlock.title"), path, QStringLiteral("KeePass (*.kdbx)"));
	if (path.isEmpty())
		return;
	bool ok = false;
	QString pw = QInputDialog::getText(this, T(m_backend, "dialog.unlock.title"), T(m_backend, "dialog.unlock.password"),
	                                   QLineEdit::Password, QString(), &ok);
	if (!ok)
		return;
	m_backend->unlock(path, pw, QString());
	pw.fill(QLatin1Char(' '));
	statusBar()->showMessage(T(m_backend, "status.unlocking"));
}

SessionPane* MainWindow::pane(std::uint64_t session)
{
	return m_panes.value(session, nullptr);
}

std::uint64_t MainWindow::currentTab() const
{
	QWidget* w = m_tabs->currentWidget();
	return w ? w->property("tab").toULongLong() : 0;
}

std::uint64_t MainWindow::currentSession() const
{
	for (auto it = m_panes.begin(); it != m_panes.end(); ++it)
		if (it.value()->isAncestorOf(QApplication::focusWidget()))
			return it.key();
	const auto tab = currentTab();
	for (const QJsonValue& t : m_workspace.value(QStringLiteral("tabs")).toArray())
		if (std::uint64_t(t.toObject().value(QStringLiteral("id")).toInteger()) == tab)
		{
			const QJsonArray s = t.toObject().value(QStringLiteral("sessions")).toArray();
			if (!s.isEmpty())
				return std::uint64_t(s.last().toInteger());
		}
	return 0;
}

QWidget* MainWindow::buildTabContent(const QJsonObject& tab)
{
	const QJsonArray sessions = tab.value(QStringLiteral("sessions")).toArray();
	const QJsonArray cells = tab.value(QStringLiteral("cells")).toArray();
	const QJsonObject info = m_workspace.value(QStringLiteral("sessions")).toObject();
	auto paneFor = [&](std::uint64_t s) {
		SessionPane* p = m_panes.value(s, nullptr);
		if (!p)
		{
			const QJsonObject i = info.value(QString::number(s)).toObject();
			p = new SessionPane(m_backend, s, i.value(QStringLiteral("title")).toString(), i.value(QStringLiteral("protocol")).toString());
			if (!i.value(QStringLiteral("state")).toString().isEmpty())
				p->setState(i.value(QStringLiteral("state")).toString(), QString());
			m_panes.insert(s, p);
		}
		return p;
	};
	// Zeilen aus den Zellen bilden; jede Zeile ist ein waagerechter Teiler.
	auto* vertical = new QSplitter(Qt::Vertical);
	vertical->setChildrenCollapsible(false);
	QMap<int, QSplitter*> rows;
	const bool single = tab.value(QStringLiteral("arrangement")).toString() == QLatin1String("single");
	for (int i = 0; i < sessions.size(); ++i)
	{
		const auto s = std::uint64_t(sessions[i].toInteger());
		SessionPane* p = paneFor(s);
		if (single && i > 0)
		{
			p->setParent(nullptr);
			p->hide();
			continue;
		}
		const int row = i < cells.size() ? cells[i].toObject().value(QStringLiteral("row")).toInt() : 0;
		if (!rows.contains(row))
		{
			auto* h = new QSplitter(Qt::Horizontal);
			h->setChildrenCollapsible(false);
			rows.insert(row, h);
			vertical->addWidget(h);
		}
		rows[row]->addWidget(p);
		p->show();
	}
	return vertical;
}

void MainWindow::rebuildTabs()
{
	m_workspace = parseObject(m_backend->workspaceJson());
	QHash<std::uint64_t, QJsonObject> tabs;
	for (const QJsonValue& t : m_workspace.value(QStringLiteral("tabs")).toArray())
		tabs.insert(std::uint64_t(t.toObject().value(QStringLiteral("id")).toInteger()), t.toObject());
	// Sitzungs-Widgets aus alten Containern lösen, damit sie nicht mit gelöscht werden.
	for (SessionPane* p : std::as_const(m_panes))
		p->setParent(nullptr);
	auto fill = [&](QTabWidget* target, const QJsonObject& window) {
		const auto active = std::uint64_t(window.value(QStringLiteral("active")).toInteger());
		while (target->count())
		{
			QWidget* w = target->widget(0);
			target->removeTab(0);
			w->deleteLater();
		}
		int activeIndex = 0;
		for (const QJsonValue& tv : window.value(QStringLiteral("tabs")).toArray())
		{
			const auto id = std::uint64_t(tv.toInteger());
			const QJsonObject tab = tabs.value(id);
			QWidget* content = buildTabContent(tab);
			content->setProperty("tab", QVariant::fromValue<qulonglong>(id));
			const int i = target->addTab(content, tab.value(QStringLiteral("title")).toString());
			if (id == active)
				activeIndex = i;
		}
		target->setCurrentIndex(activeIndex);
	};
	const QJsonArray windows = m_workspace.value(QStringLiteral("windows")).toArray();
	QSet<std::uint64_t> seen;
	for (int wi = 0; wi < windows.size(); ++wi)
	{
		const QJsonObject w = windows[wi].toObject();
		const auto wid = std::uint64_t(w.value(QStringLiteral("id")).toInteger());
		if (wi == 0)
		{
			fill(m_tabs, w);
			continue;
		}
		seen.insert(wid);
		QTabWidget* tw = m_windowTabs.value(wid, nullptr);
		if (!tw || !m_windows.value(wid))
		{
			auto* win = new QWidget(nullptr, Qt::Window);
			win->setAttribute(Qt::WA_DeleteOnClose);
			win->setWindowTitle(T(m_backend, "app.title"));
			win->resize(900, 650);
			auto* lay = new QVBoxLayout(win);
			lay->setContentsMargins(0, 0, 0, 0);
			tw = new QTabWidget(win);
			tw->setDocumentMode(true);
			tw->tabBar()->setContextMenuPolicy(Qt::CustomContextMenu);
			connect(tw->tabBar(), &QTabBar::customContextMenuRequested, this, [this, tw](const QPoint& p) { tabContextMenu(tw, p); });
			lay->addWidget(tw);
			m_windows.insert(wid, win);
			m_windowTabs.insert(wid, tw);
			win->show();
		}
		fill(tw, w);
	}
	for (auto it = m_windows.begin(); it != m_windows.end();)
	{
		if (!seen.contains(it.key()))
		{
			if (it.value())
				it.value()->close();
			m_windowTabs.remove(it.key());
			it = m_windows.erase(it);
		}
		else
			++it;
	}
	updateStatus();
}

QWidget* MainWindow::detachedWindow() const
{
	for (const auto& w : m_windows)
		if (w)
			return w;
	return nullptr;
}

void MainWindow::updateStatus()
{
	QString state;
	const auto s = currentSession();
	if (auto* p = m_panes.value(s, nullptr))
		state = p->title() + QStringLiteral(": ") + m_backend->text(QStringLiteral("state.") + (p->state().isEmpty() ? QStringLiteral("connecting") : p->state()));
	statusBar()->showMessage(T(m_backend, "status.sessions").arg(m_panes.size()) + (state.isEmpty() ? QString() : QStringLiteral(" — ") + state));
}

void MainWindow::poll()
{
	const QString json = m_backend->poll();
	if (json.size() > 2)
	{
		for (const QJsonValue& v : QJsonDocument::fromJson(json.toUtf8()).array())
		{
			const QJsonObject e = v.toObject();
			const QString t = e.value(QStringLiteral("t")).toString();
			const auto s = std::uint64_t(e.value(QStringLiteral("s")).toInteger());
			SessionPane* p = pane(s);
			if (t == QLatin1String("state"))
			{
				if (p)
					p->setState(e.value(QStringLiteral("state")).toString(), e.value(QStringLiteral("detail")).toString());
				updateStatus();
			}
			else if (t == QLatin1String("shared_frame") && p && p->frame())
				p->frame()->attach(e.value(QStringLiteral("path")).toString(), e.value(QStringLiteral("w")).toInt(), e.value(QStringLiteral("h")).toInt());
			else if (t == QLatin1String("frame") && p && p->frame())
				p->frame()->damage(e.value(QStringLiteral("x")).toInt(), e.value(QStringLiteral("y")).toInt(), e.value(QStringLiteral("w")).toInt(),
				                   e.value(QStringLiteral("h")).toInt());
			else if (t == QLatin1String("terminal"))
				m_dirtyTerminals.insert(s);
			else if (t == QLatin1String("question"))
			{
				QMessageBox box(QMessageBox::Warning, T(m_backend, "app.title"), e.value(QStringLiteral("text")).toString(), QMessageBox::NoButton, this);
				box.setInformativeText(e.value(QStringLiteral("detail")).toString());
				QPushButton* yes = box.addButton(T(m_backend, "dialog.question.accept"), QMessageBox::AcceptRole);
				QPushButton* no = box.addButton(T(m_backend, "dialog.question.reject"), QMessageBox::RejectRole);
				box.setDefaultButton(no);
				box.exec();
				m_backend->answer(s, std::uint32_t(e.value(QStringLiteral("q")).toInt()), box.clickedButton() == yes);
			}
			else if (t == QLatin1String("notice"))
			{
				if (p)
					p->setNotice(e.value(QStringLiteral("text")).toString());
				statusBar()->showMessage(e.value(QStringLiteral("text")).toString(), 8000);
			}
			else if (t == QLatin1String("clipboard"))
				QGuiApplication::clipboard()->setText(e.value(QStringLiteral("text")).toString());
			else if (t == QLatin1String("clipboard_read_request"))
			{
				const bool allow = QMessageBox::question(this, T(m_backend, "app.title"), T(m_backend, "clipboard.read_request")) == QMessageBox::Yes;
				m_backend->clipboardReadAnswer(s, allow, QGuiApplication::clipboard()->text());
			}
			else if (t == QLatin1String("title") && p)
				p->setToolTip(e.value(QStringLiteral("title")).toString());
			else if (t == QLatin1String("unlocked"))
			{
				reloadTree();
				statusBar()->showMessage(T(m_backend, "status.unlocked"), 5000);
			}
			else if (t == QLatin1String("unlock_failed"))
				QMessageBox::warning(this, T(m_backend, "dialog.unlock.title"), e.value(QStringLiteral("reason")).toString());
		}
	}
	for (const auto s : std::as_const(m_dirtyTerminals))
		if (auto* p = pane(s); p && p->terminal())
			p->terminal()->refresh();
	m_dirtyTerminals.clear();
}

void MainWindow::showCredentialChoiceForScreenshot()
{
	QJsonArray choices;
	choices.append(QJsonObject{ { QStringLiteral("name"), QStringLiteral("admin") }, { QStringLiteral("title"), QStringLiteral("Verwaltung Standort Nord") } });
	choices.append(QJsonObject{ { QStringLiteral("name"), QStringLiteral("user") }, { QStringLiteral("title"), QStringLiteral("Benutzerkonto") } });
	auto* d = new CredentialDialog(m_backend, choices, this);
	d->setAttribute(Qt::WA_DeleteOnClose);
	d->setObjectName(QStringLiteral("credentialDialog"));
	d->show();
}

void MainWindow::detachTabAt(int index)
{
	if (QWidget* w = m_tabs->widget(index))
		if (m_backend->detachTab(w->property("tab").toULongLong()))
			rebuildTabs();
}

void MainWindow::paintEvent(QPaintEvent* e)
{
	QMainWindow::paintEvent(e);
	if (firstPaintMs < 0)
		firstPaintMs = startClock.elapsed();
}

void MainWindow::closeEvent(QCloseEvent* e)
{
	for (auto& w : m_windows)
		if (w)
			w->close();
	QMainWindow::closeEvent(e);
}
