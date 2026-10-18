#include "mainwindow.h"
#include "kbcapture.h"
#include "titlebar.h"
#include "exporttab.h"
#include "appearance.h"
#include <algorithm>

#include "editor.h"
#include "remotejoe-ui/src/bridge.cxxqt.h"
#include "views.h"

#include <QtCore/QDateTime>
#include <QtCore/QDir>
#include <QtCore/QJsonArray>
#include <QtCore/QJsonDocument>
#include <QtGui/QAction>
#include <QtCore/QCryptographicHash>
#include <QtCore/QMimeData>
#include <QtGui/QClipboard>
#include <QtWidgets/QInputDialog>
#include <QtGui/QDropEvent>
#include <QtGui/QCloseEvent>
#include <QtGui/QFontDatabase>
#include <QtGui/QGuiApplication>
#include <QtGui/QKeySequence>
#include <QtGui/QPainter>
#include <QtGui/QPainterPath>
#include <QtGui/QShortcut>
#include <QtSvg/QSvgRenderer>
#include <QtWidgets/QApplication>
#include <QtWidgets/QAbstractButton>
#include <QtWidgets/QCheckBox>
#include <QtWidgets/QToolButton>
#include <QtGui/QScreen>
#include <QtGui/QResizeEvent>
#include <QtWidgets/QComboBox>
#include <QtCore/QPointer>
#include <QtWidgets/QFileDialog>
#include <QtWidgets/QFormLayout>
#include <QtWidgets/QHBoxLayout>
#include <QtWidgets/QHeaderView>
#include <QtWidgets/QLabel>
#include <QtWidgets/QMenu>
#include <QtWidgets/QScrollBar>
#include <functional>
#include <QtWidgets/QMessageBox>
#include <QtWidgets/QMenuBar>
#include <QtWidgets/QSplitter>
#include <QtWidgets/QStatusBar>
#include <QtWidgets/QStyleFactory>
#include <QtGui/QStyleHints>
#include <QtGui/QDesktopServices>
#include <QtGui/QWindow>
#include <QtWidgets/QTabBar>
#include <QtGui/QHelpEvent>
#include <QtWidgets/QToolTip>
#include <QtWidgets/QPlainTextEdit>
#include <QtWidgets/QProgressBar>
#include <QtWidgets/QPushButton>
#include <QtWidgets/QTableWidget>
#include <QtWidgets/QVBoxLayout>
#include <QtCore/QAbstractNativeEventFilter>
#ifdef Q_OS_LINUX
#include <QtDBus/QDBusConnection>
#endif
#ifdef Q_OS_WIN
#include <windows.h>
#endif

QElapsedTimer MainWindow::startClock;

namespace {

constexpr int RoleId = Qt::UserRole + 1;
constexpr int RoleFolder = Qt::UserRole + 2;
/// Gast eines Hypervisors (Kennung), Ziel des Hypervisors, Konsolenadresse.
constexpr int RoleGuest = Qt::UserRole + 3;
constexpr int RoleGuestTarget = Qt::UserRole + 4;
constexpr int RoleConsoleHost = Qt::UserRole + 5;
/// Platzhalter unter einem Hypervisor, bis die Gäste geladen sind.
constexpr int RolePlaceholder = Qt::UserRole + 6;
constexpr int RoleProtocol = Qt::UserRole + 7;
/// Fach „Makros“ im Baum: 1 = das Fach selbst, 2 = ein Makro (RoleId = Makro-ID).
constexpr int RoleMacro = Qt::UserRole + 20;

QString T(RjBackend* b, const char* key)
{
	return b->text(QString::fromLatin1(key));
}

QJsonObject parseObject(const QString& s)
{
	return QJsonDocument::fromJson(s.toUtf8()).object();
}

/// Symbol je Protokoll (SVG mit Kürzel), optional mit Ampelpunkt.
/// Zeichnet ein kleines SVG-Symbol (Körper mit dem Platzhalter COLOR) in der Textfarbe.
QPixmap svgPixmap(const char* body, const QColor& color, int px)
{
	const QString svg = QStringLiteral("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16'>") +
	                    QString::fromLatin1(body).replace(QLatin1String("COLOR"), color.name()) + QStringLiteral("</svg>");
	QSvgRenderer renderer(svg.toUtf8());
	const qreal dpr = qApp->devicePixelRatio();
	QPixmap pm(QSize(px, px) * dpr);
	pm.setDevicePixelRatio(dpr);
	pm.fill(Qt::transparent);
	QPainter p(&pm);
	renderer.render(&p, QRectF(0, 0, px, px));
	return pm;
}

const char* const kSvgTarget = "<rect x='1.5' y='2.5' width='13' height='8.5' rx='1' fill='none' stroke='COLOR' stroke-width='1.3'/>"
                               "<path d='M5 14h6M8 11v3' stroke='COLOR' stroke-width='1.3' fill='none' stroke-linecap='round'/>";
const char* const kSvgClock = "<circle cx='8' cy='8' r='6' fill='none' stroke='COLOR' stroke-width='1.3'/>"
                              "<path d='M8 4.5V8l2.5 1.5' stroke='COLOR' stroke-width='1.3' fill='none' stroke-linecap='round' stroke-linejoin='round'/>";
const char* const kSvgUser = "<circle cx='8' cy='5.5' r='2.7' fill='none' stroke='COLOR' stroke-width='1.3'/>"
                             "<path d='M2.8 14c0-3 2.3-4.8 5.2-4.8s5.2 1.8 5.2 4.8' stroke='COLOR' stroke-width='1.3' fill='none' stroke-linecap='round'/>";

QString protocolName(const QString& p)
{
	static const QHash<QString, QString> names = { { QStringLiteral("ssh"), QStringLiteral("SSH") },     { QStringLiteral("rdp"), QStringLiteral("RDP") },
		                                           { QStringLiteral("vnc"), QStringLiteral("VNC") },     { QStringLiteral("web"), QStringLiteral("Web") },
		                                           { QStringLiteral("psrp"), QStringLiteral("PowerShell") }, { QStringLiteral("proxmox"), QStringLiteral("Proxmox") },
		                                           { QStringLiteral("hyperv"), QStringLiteral("Hyper-V") } };
	return names.value(p, p.toUpper());
}

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
		const char* glyph; // weißes Piktogramm im Koordinatenraum 32 × 32
		const char* color;
	};
	// Gemeinsame Strichart der Piktogramme.
	#define RJ_G(body) "<g fill='none' stroke='#ffffff' stroke-width='2.2' stroke-linecap='round' stroke-linejoin='round'>" body "</g>"
	static const P table[] = {
		// RDP: Bildschirm mit Fuß
		{ "rdp", RJ_G("<rect x='6' y='7' width='20' height='13' rx='1.5'/><path d='M12 25h8M16 20v5'/>"), "#0b5cad" },
		// SSH: Eingabeaufforderung „>_“
		{ "ssh", RJ_G("<path d='M8 10l7 6-7 6'/><path d='M17 23h8'/>"), "#1b6e20" },
		// VNC: Bildschirm mit Mauszeiger
		{ "vnc", RJ_G("<rect x='5' y='7' width='19' height='13' rx='1.5'/><path d='M11 25h7'/><path d='M19 14l7 4-3 1 2 4-2 1-2-4-2 2z' fill='#ffffff'/>"), "#7a3e9d" },
		// Web: Weltkugel
		{ "web", RJ_G("<circle cx='16' cy='16' r='10'/><ellipse cx='16' cy='16' rx='4.5' ry='10'/><path d='M6 16h20M7.5 10.5h17M7.5 21.5h17'/>"), "#8a5300" },
		// PowerShell: Fenster mit Pfeilspitze und Strich
		{ "psrp", RJ_G("<path d='M5 24L8 8h19l-3 16z'/><path d='M12 13l5 4-6 4'/><path d='M17 22h5'/>"), "#005f73" },
		// Proxmox: Würfel
		{ "proxmox", RJ_G("<path d='M16 5l9 5v11l-9 6-9-6V10z'/><path d='M7 10l9 5 9-5M16 15v12'/>"), "#b03a00" },
		// Hyper-V: zwei Server übereinander
		{ "hyperv", RJ_G("<rect x='6' y='6' width='20' height='8' rx='1.5'/><rect x='6' y='18' width='20' height='8' rx='1.5'/><path d='M10 10h.1M10 22h.1M15 10h8M15 22h8'/>"), "#3a4a8c" },
		// Makro: Abspielzeichen
		{ "macro", RJ_G("<path d='M11 8l14 8-14 8z' fill='#ffffff'/>"), "#6a1b9a" },
	};
	QString svg;
	if (folder)
		svg = QStringLiteral("<svg xmlns='http://www.w3.org/2000/svg' width='32' height='32'>"
		                     "<path d='M2 8h11l3 3h14v17H2z' fill='#c9a227' stroke='#6b5410' stroke-width='1.5'/></svg>");
	else
	{
		QString glyph = QStringLiteral(RJ_G("<path d='M12 12a4 4 0 1 1 6 3.5c-1.5 1-2 1.5-2 3'/><path d='M16 23h.1'/>")), color = QStringLiteral("#555555");
		for (const P& p : table)
			if (protocol == QLatin1String(p.name))
			{
				glyph = QLatin1String(p.glyph);
				color = QLatin1String(p.color);
			}
		svg = QStringLiteral("<svg xmlns='http://www.w3.org/2000/svg' width='32' height='32'>"
		                     "<rect x='1' y='3' width='30' height='26' rx='5' fill='%1'/>%2</svg>")
		        .arg(color, glyph);
	}
	#undef RJ_G
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
		const QString proto = n.value(QStringLiteral("protocol")).toString();
		item->setData(proto, RoleProtocol);
		parent->appendRow(item);
		addNodes(item, n.value(QStringLiteral("children")).toArray());
		if (proto == QStringLiteral("proxmox") || proto == QStringLiteral("hyperv"))
		{
			auto* ph = new QStandardItem(QStringLiteral("…"));
			ph->setEditable(false);
			ph->setData(true, RolePlaceholder);
			ph->setData(true, RoleFolder);
			item->appendRow(ph);
		}
	}
}

#ifdef Q_OS_WIN
/// Energiestatus-Meldungen des Fensters (Aufwachen aus dem Standby).
class PowerFilter : public QAbstractNativeEventFilter
{
public:
	explicit PowerFilter(std::function<void()> resumed)
	  : m_resumed(std::move(resumed))
	{
	}
	bool nativeEventFilter(const QByteArray& type, void* message, qintptr*) override
	{
		if (type == "windows_generic_MSG")
		{
			const MSG* m = static_cast<const MSG*>(message);
			if (m->message == WM_POWERBROADCAST && (m->wParam == PBT_APMRESUMEAUTOMATIC || m->wParam == PBT_APMRESUMESUSPEND))
				m_resumed();
		}
		return false;
	}

private:
	std::function<void()> m_resumed;
};
#endif

/// Baum mit Ziehen und Ablegen: verschoben wird im Index, nicht im Modell.
class RjTree : public QTreeView
{
public:
	using QTreeView::QTreeView;
	/// Lage: 0 auf, 1 über, 2 unter dem Eintrag, 3 freie Fläche.
	std::function<void(const QModelIndex& source, const QModelIndex& target, int position)> onDrop;

protected:
	void dropEvent(QDropEvent* e) override
	{
		const QModelIndex src = currentIndex();
		const QModelIndex dst = indexAt(e->position().toPoint());
		const int pos = int(dropIndicatorPosition());
		e->setDropAction(Qt::IgnoreAction);
		e->accept();
		if (onDrop && src.isValid())
			onDrop(src, dst, pos);
	}
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
	auto* t = new PatchdayTab(m_backend, ids, m_tabs);
	addToolTab(t, T(m_backend, "patch.title"));
}

QString MainWindow::patchdayMeasurement() const
{
	auto* p = findChild<PatchdayTab*>();
	return p ? QStringLiteral("patchday_neuzeichnen_max_ms=%1\npatchday_neuzeichnen_anzahl=%2\n").arg(p->maxRefreshMs).arg(p->refreshes) : QString();
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

namespace {
/// Breite des Randes, an dem das Fenster in der Größe verändert wird (Pixel).
constexpr int kResizeBorder = 5;

/// Kleiner Schiebeschalter für die Fußzeile.
class SwitchButton : public QAbstractButton
{
public:
	explicit SwitchButton(QWidget* parent)
	  : QAbstractButton(parent)
	{
		setCheckable(true);
		setFixedSize(26, 12);
		setCursor(Qt::PointingHandCursor);
		setFocusPolicy(Qt::NoFocus);
	}

protected:
	void paintEvent(QPaintEvent*) override
	{
		QPainter p(this);
		p.setRenderHint(QPainter::Antialiasing);
		const QRectF r = QRectF(rect()).adjusted(0.5, 0.5, -0.5, -0.5);
		p.setPen(Qt::NoPen);
		p.setBrush(isChecked() ? palette().highlight() : palette().mid());
		p.drawRoundedRect(r, r.height() / 2, r.height() / 2);
		const qreal d = r.height() - 4;
		p.setBrush(palette().base());
		p.drawEllipse(QRectF(isChecked() ? r.right() - d - 2 : r.left() + 2, r.top() + 2, d, d));
	}
};
} // namespace

MainWindow::MainWindow(RjBackend* backend, QWidget* parent)
  : QMainWindow(parent)
  , m_backend(backend)
{
	const QString version = m_backend->appVersion();
	setWindowTitle(QStringLiteral("%1 V %2").arg(T(m_backend, "app.title"), version));
	resize(1280, 800);
	// Eigene Titelleiste (links Logo, Name, Version): gleiches Aussehen unter Ubuntu und Windows.
	setWindowFlag(Qt::FramelessWindowHint, true);
	setMouseTracking(true);
	setContentsMargins(kResizeBorder, kResizeBorder, kResizeBorder, kResizeBorder);
	{
		auto* top = new QWidget(this);
		auto* tl = new QVBoxLayout(top);
		tl->setContentsMargins(0, 0, 0, 0);
		tl->setSpacing(0);
		m_titleBar = new TitleBar(this, T(m_backend, "app.title"), version, top);
		m_menuBar = new QMenuBar(top);
		tl->addWidget(m_titleBar);
		tl->addWidget(m_menuBar);
		setMenuWidget(top);
	}

	auto* left = new QWidget(this);
	m_leftPane = left;
	auto* leftOuter = new QVBoxLayout(left);
	leftOuter->setContentsMargins(0, 0, 0, 0);
	leftOuter->setSpacing(0);
	// Im Platzsparmodus mit angepinntem Baum steht hier Logo, Name und Version über dem Baum.
	m_brandSlot = new QWidget(left);
	m_brandSlotLay = new QVBoxLayout(m_brandSlot);
	m_brandSlotLay->setContentsMargins(0, 0, 0, 0);
	m_brandSlot->hide();
	m_brandSlot->setSizePolicy(QSizePolicy::Preferred, QSizePolicy::Fixed);
	auto* leftInner = new QWidget(left);
	m_leftInner = leftInner;
	leftOuter->addWidget(m_brandSlot);
	leftOuter->addWidget(leftInner, 1);
	leftOuter->addStretch(0);
	auto* leftLay = new QVBoxLayout(leftInner);
	leftLay->setContentsMargins(4, 4, 4, 4);
	m_search = new QLineEdit(leftInner);
	m_search->setPlaceholderText(T(m_backend, "search.placeholder"));
	m_search->setClearButtonEnabled(true);
	m_search->setMinimumHeight(32);
	m_search->setAccessibleName(T(m_backend, "search.placeholder"));
	m_viewBox = new QComboBox(leftInner);
	m_viewBox->setAccessibleName(T(m_backend, "view.label"));
	m_bar = new ActionBar(m_backend, leftInner);
	auto* tree = new RjTree(leftInner);
	m_tree = tree;
	tree->setDragEnabled(true);
	tree->setAcceptDrops(true);
	tree->setDropIndicatorShown(true);
	tree->setDragDropMode(QAbstractItemView::InternalMove);
	tree->onDrop = [this](const QModelIndex& src, const QModelIndex& dst, int pos) {
		if (src.data(RoleGuest).isValid() || src.data(RolePlaceholder).toBool() || dst.data(RoleGuest).isValid() || dst.data(RolePlaceholder).toBool() ||
		    src.data(RoleMacro).toInt() || dst.data(RoleMacro).toInt())
			return;
		QString parent;
		QJsonValue at;
		if (!dst.isValid() || pos == 3)
			parent = QString();
		else if (pos == 0 && dst.data(RoleFolder).toBool())
			parent = dst.data(RoleId).toString();
		else
		{
			parent = dst.parent().isValid() ? dst.parent().data(RoleId).toString() : QString();
			at = dst.row() + (pos == 2 ? 1 : 0);
		}
		QJsonObject cmd{ { QStringLiteral("op"), QStringLiteral("move") }, { QStringLiteral("id"), src.data(RoleId).toString() } };
		if (!parent.isEmpty())
			cmd.insert(QStringLiteral("parent"), parent);
		if (!at.isUndefined())
			cmd.insert(QStringLiteral("pos"), at);
		const QString id = src.data(RoleId).toString();
		QTimer::singleShot(0, this, [this, cmd, id] {
			showEditResult(parseObject(edit(cmd)));
			selectId(id);
		});
	};
	m_model = new QStandardItemModel(this);
	m_tree->setModel(m_model);
	m_tree->setHeaderHidden(true);
	m_tree->setUniformRowHeights(true);
	m_tree->setIconSize(QSize(22, 22));
	m_tree->setContextMenuPolicy(Qt::CustomContextMenu);
	m_tree->setSelectionMode(QAbstractItemView::ExtendedSelection);
	// Hinweis im leeren Baum: sagt, wie man ein erstes Ziel oder einen Ordner anlegt.
	m_emptyHint = new QWidget(m_tree->viewport());
	{
		auto* el = new QVBoxLayout(m_emptyHint);
		el->setContentsMargins(12, 12, 12, 12);
		el->addStretch(1);
		m_emptyLabel = new QLabel(T(m_backend, "tree.empty"), m_emptyHint);
		m_emptyLabel->setWordWrap(true);
		m_emptyLabel->setAlignment(Qt::AlignCenter);
		el->addWidget(m_emptyLabel);
		m_emptyButton = new QPushButton(T(m_backend, "tree.empty.add"), m_emptyHint);
		el->addWidget(m_emptyButton, 0, Qt::AlignHCenter);
		el->addStretch(2);
		// Ohne entsperrte Schlüsseldatenbank führt der Knopf zum Entsperren, sonst zum Hinzufügen.
		connect(m_emptyButton, &QPushButton::clicked, this, [this] {
			if (m_backend->isVaultOpen())
				popupAddMenu();
			else
				showUnlock(false);
		});
		m_emptyHint->hide();
		m_tree->viewport()->installEventFilter(this);
	}
	m_tree->setAccessibleName(QStringLiteral("Verbindungsbaum"));
	leftLay->addWidget(m_viewBox);
	leftLay->addWidget(m_search);
	leftLay->addWidget(m_bar);
	leftLay->addWidget(m_tree, 1);

	m_tabs = new QTabWidget(this);
	m_tabs->setTabsClosable(true);
	m_tabs->setMovable(true);
	m_tabs->setDocumentMode(true);
	m_tabs->tabBar()->setContextMenuPolicy(Qt::CustomContextMenu);
	configureTabBar(m_tabs);

	auto* split = new QSplitter(Qt::Horizontal, this);
	split->addWidget(left);
	split->addWidget(m_tabs);
	split->setStretchFactor(1, 1);
	split->setSizes({ 300, 980 });
	m_split = split;
	setCentralWidget(split);

	buildMenus();
	// Hamburger-Menü (Platzsparmodus): dieselben Menüs; die Kürzel gelten auch bei verborgener Menüzeile.
	m_burgerMenu = new QMenu(this);
	for (QAction* a : m_menuBar->actions())
		if (a->menu())
			m_burgerMenu->addMenu(a->menu());
	{
		std::function<void(QMenu*)> carry = [&](QMenu* m) {
			for (QAction* a : m->actions())
			{
				if (a->menu())
					carry(a->menu());
				else if (!a->shortcut().isEmpty())
					addAction(a);
			}
		};
		for (QAction* a : m_menuBar->actions())
			if (a->menu())
				carry(a->menu());
	}
	m_tabs->tabBar()->installEventFilter(this);
	m_tabs->installEventFilter(this);
	m_fitTimer.setSingleShot(true);
	m_fitTimer.setInterval(300);
	connect(&m_fitTimer, &QTimer::timeout, this, &MainWindow::fitToScreen);
	connect(qApp, &QGuiApplication::screenAdded, this, [this] { screenLayoutChanged(); });
	connect(qApp, &QGuiApplication::screenRemoved, this, [this] { screenLayoutChanged(); });
	connect(qApp, &QGuiApplication::primaryScreenChanged, this, [this] { screenLayoutChanged(); });
#if QT_VERSION >= QT_VERSION_CHECK(6, 5, 0)
	// Hell/Dunkel des Desktops live übernehmen (ab Qt 6.5; unter Ubuntu 24.04 mit Qt 6.4 gilt es beim Start).
	connect(QGuiApplication::styleHints(), &QStyleHints::colorSchemeChanged, this, [this] { applyAppearance(); });
#endif
	FrameView::setFocusFrame(parseObject(m_backend->appearanceJson()).value(QStringLiteral("focus_frame")).toBool(true));
	FrameView::s_keyCapture = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("settings") } })).value(QStringLiteral("keyboard_capture")).toBool(true);
	FrameView::s_notice = [this](const QString& m) { statusBar()->showMessage(m, 8000); };
	statusBar()->showMessage(T(m_backend, "status.ready"));
	// Fußzeile rechts: womit, wohin, seit wann, als wer.
	m_connInfo = new QWidget(this);
	{
		auto* l = new QHBoxLayout(m_connInfo);
		l->setContentsMargins(0, 0, 4, 0);
		l->setSpacing(3);
		auto add = [&](QLabel*& icon, QLabel*& text, const char* tip) {
			icon = new QLabel(m_connInfo);
			icon->setFixedSize(11, 11);
			icon->setScaledContents(true);
			text = new QLabel(m_connInfo);
			text->setToolTip(T(m_backend, tip));
			icon->setToolTip(T(m_backend, tip));
			l->addWidget(icon);
			l->addWidget(text);
			l->addSpacing(6);
		};
		add(m_ciProtoIcon, m_ciProto, "ci.tip.type");
		add(m_ciTargetIcon, m_ciTarget, "ci.tip.target");
		add(m_ciSinceIcon, m_ciSince, "ci.tip.since");
		add(m_ciUserIcon, m_ciUser, "ci.tip.user");
		m_ciTarget->setTextInteractionFlags(Qt::TextSelectableByMouse);
	}
	statusBar()->addPermanentWidget(m_connInfo);
	m_connInfo->hide();
	connect(&m_ciTimer, &QTimer::timeout, this, &MainWindow::updateConnInfo);
	m_ciTimer.start(1000);

	// Eingabetaste und Doppelklick verbinden; die Eigenschaften öffnen F4, Alt+Eingabe und das Kontextmenü.
	auto treeKey = [this, left](const QKeySequence& k, auto fn) {
		auto* s = new QShortcut(k, left);
		s->setContext(Qt::WidgetWithChildrenShortcut);
		connect(s, &QShortcut::activated, this, fn);
	};
	auto activate = [this] {
		const QModelIndex i = m_tree->currentIndex();
		if (!m_tree->hasFocus() || !i.isValid())
			return;
		if (i.data(RoleMacro).toInt() == 2)
		{
			openMacroTab(i.data(RoleId).toString());
			return;
		}
		if (i.data(RoleMacro).toInt() == 1)
		{
			m_tree->setExpanded(i, !m_tree->isExpanded(i));
			return;
		}
		if (i.data(RoleGuest).isValid())
			openGuestConsole(i);
		else if (!i.data(RoleFolder).toBool())
			connectSelected();
		else
			m_tree->setExpanded(i, !m_tree->isExpanded(i));
	};
	treeKey(QKeySequence(Qt::Key_Return), activate);
	treeKey(QKeySequence(Qt::Key_Enter), activate);
	connect(m_tree, &QTreeView::expanded, this, [this](const QModelIndex& i) {
		if (i.data(RoleMacro).toInt() == 1)
			m_macroShelfOpen = true;
	});
	connect(m_tree, &QTreeView::collapsed, this, [this](const QModelIndex& i) {
		if (i.data(RoleMacro).toInt() == 1)
			m_macroShelfOpen = false;
	});
	connect(m_tree, &QTreeView::doubleClicked, this, [this](const QModelIndex& i) {
		if (i.data(RoleMacro).toInt() == 2)
		{
			openMacroTab(i.data(RoleId).toString());
			return;
		}
		if (i.data(RoleGuest).isValid())
			openGuestConsole(i);
		else if (!i.data(RolePlaceholder).toBool() && !i.data(RoleFolder).toBool())
			connectSelected();
		// Ordner klappen beim Doppelklick wie üblich auf oder zu.
	});
	treeKey(QKeySequence(Qt::Key_F4), [this] { if (!currentId().isEmpty()) openProperties(currentId()); });
	treeKey(QKeySequence(QStringLiteral("Alt+Return")), [this] { if (!currentId().isEmpty()) openProperties(currentId()); });
	treeKey(QKeySequence(QStringLiteral("Ctrl+N")), [this] { popupAddMenu(); });
	treeKey(QKeySequence(Qt::Key_Insert), [this] { popupAddMenu(); });
	treeKey(QKeySequence(Qt::Key_F2), [this] { renameSelected(); });
	treeKey(QKeySequence(Qt::Key_Delete), [this] {
		const QModelIndex i = m_tree->currentIndex();
		if (i.isValid() && i.data(RoleMacro).toInt() == 2)
			deleteMacro(i.data(RoleId).toString(), i.data(Qt::DisplayRole).toString());
		else
			deleteSelected();
	});
	treeKey(QKeySequence(QStringLiteral("Ctrl+D")), [this] { duplicateSelected(); });
	treeKey(QKeySequence(QStringLiteral("Ctrl+Shift+M")), [this] { moveSelected(); });
	treeKey(QKeySequence(QStringLiteral("Alt+Up")), [this] { reorderSelected(-1); });
	treeKey(QKeySequence(QStringLiteral("Alt+Down")), [this] { reorderSelected(1); });
	treeKey(QKeySequence(QStringLiteral("Ctrl+B")), [this] { toggleFavorite(); });
	treeKey(QKeySequence(QStringLiteral("Ctrl+Z")), [this] { undo(); });
	connect(m_viewBox, &QComboBox::activated, this, [this](int) {
		edit({ { QStringLiteral("op"), QStringLiteral("set_view") }, { QStringLiteral("view"), m_viewBox->currentData().toString() } });
		reloadTree();
	});
	m_overlay = new UnlockOverlay(m_backend, this);
	connect(m_overlay, &UnlockOverlay::unlockRequested, this, [this](const QString& path, const QString& pw, const QString& keyFile, bool remember) {
		m_backend->unlock(path, pw, keyFile, remember);
		statusBar()->showMessage(T(m_backend, "status.unlocking"));
	});
	connect(m_overlay, &UnlockOverlay::createRequested, this, [this](const QString& path, const QString& pw, const QString& keyFile, bool makeKey, bool remember) {
		m_backend->createVault(path, pw, keyFile, makeKey, remember);
		statusBar()->showMessage(T(m_backend, "status.creating"));
	});
	// Hintergrundaufgaben in der Statuszeile: Fortschritt und Abbruch.
	m_jobsBox = new QWidget(this);
	auto* jl = new QHBoxLayout(m_jobsBox);
	jl->setContentsMargins(0, 0, 0, 0);
	auto* busy = new QProgressBar(m_jobsBox);
	busy->setRange(0, 0);
	busy->setMaximumWidth(120);
	busy->setMaximumHeight(8);
	m_jobsLabel = new QLabel(m_jobsBox);
	auto* cancelJob = new QPushButton(T(m_backend, "jobs.cancel"), m_jobsBox);
	cancelJob->setObjectName(QStringLiteral("cancelJob"));
	connect(cancelJob, &QPushButton::clicked, this, [this] {
		const QJsonArray jobs = QJsonDocument::fromJson(edit({ { QStringLiteral("op"), QStringLiteral("jobs") } }).toUtf8()).array();
		if (!jobs.isEmpty())
			edit({ { QStringLiteral("op"), QStringLiteral("cancel_job") }, { QStringLiteral("job"), jobs.first().toObject().value(QStringLiteral("id")) } });
		updateJobs();
	});
	jl->addWidget(busy);
	jl->addWidget(m_jobsLabel);
	jl->addWidget(cancelJob);
	statusBar()->addPermanentWidget(m_jobsBox);
	m_jobsBox->hide();
	// Fußzeile (kompakt): Baum-Knopf, Hinweise der vorderen Sitzung in der Mitte, rechts der Schalter.
	statusBar()->setSizeGripEnabled(false);
	{
		// Fußzeile nur halb so hoch wie üblich: kleinere Schrift, keine Ränder.
		QFont sf = statusBar()->font();
		sf.setPointSizeF(sf.pointSizeF() * 0.8);
		statusBar()->setFont(sf);
		statusBar()->setContentsMargins(0, 0, 0, 0);
	}
	m_treeButton = new QToolButton(this);
	m_treeButton->setAutoRaise(true);
	m_treeButton->setFocusPolicy(Qt::NoFocus);
	m_treeButton->setObjectName(QStringLiteral("statusTreeButton"));
	connect(m_treeButton, &QToolButton::clicked, this, [this] { setTreeVisible(m_treeHidden); });
	statusBar()->insertPermanentWidget(0, m_treeButton);
	m_noticeLabel = new QLabel(this);
	m_noticeLabel->setAlignment(Qt::AlignLeft | Qt::AlignVCenter);
	m_noticeLabel->setContentsMargins(8, 0, 8, 0);
	m_noticeLabel->setObjectName(QStringLiteral("statusNotice"));
	m_noticeLabel->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
	statusBar()->insertPermanentWidget(1, m_noticeLabel, 1);
	{
		auto* box = new QWidget(this);
		auto* bl = new QHBoxLayout(box);
		bl->setContentsMargins(6, 0, 4, 0);
		bl->setSpacing(5);
		auto* lab = new QLabel(T(m_backend, "status.compact"), box);
		m_compactSwitch = new SwitchButton(box);
		bl->addWidget(lab);
		bl->addWidget(m_compactSwitch);
		box->setToolTip(T(m_backend, "status.compact.tip"));
		m_compactSwitch->setAccessibleName(T(m_backend, "status.compact"));
		connect(m_compactSwitch, &QAbstractButton::toggled, this, [this](bool on) { setCompact(on); });
		statusBar()->addPermanentWidget(box);
	}
	setTreeVisible(true);
	// Leerlauf: Eingaben überall im Programm zählen.
	m_activityClock.start();
	qApp->installEventFilter(this);
#ifdef Q_OS_LINUX
	QDBusConnection::systemBus().connect(QStringLiteral("org.freedesktop.login1"), QStringLiteral("/org/freedesktop/login1"),
	                                     QStringLiteral("org.freedesktop.login1.Manager"), QStringLiteral("PrepareForSleep"), this,
	                                     SLOT(onPrepareForSleep(bool)));
#endif
#ifdef Q_OS_WIN
	static PowerFilter power([this] { systemResumed(); });
	qApp->installNativeEventFilter(&power);
#endif
	connect(m_tree, &QTreeView::expanded, this, [this](const QModelIndex& i) {
		const QModelIndex first = m_model->index(0, 0, i);
		if (first.isValid() && first.data(RolePlaceholder).toBool())
			loadGuests(i);
	});
	connect(m_tree, &QTreeView::customContextMenuRequested, this, &MainWindow::treeContextMenu);
	m_filterTimer.setSingleShot(true);
	m_filterTimer.setInterval(150);
	connect(m_search, &QLineEdit::textChanged, this, [this] { m_filterTimer.start(); });
	connect(&m_filterTimer, &QTimer::timeout, this, &MainWindow::reloadTree);
	connect(m_tabs, &QTabWidget::tabCloseRequested, this, [this](int i) {
		if (auto* st = qobject_cast<SnmpTab*>(m_tabs->widget(i)))
		{
			st->wipeSecrets();
			m_tabs->removeTab(i);
			st->deleteLater();
			return;
		}
		if (auto* tt = qobject_cast<ToolTab*>(m_tabs->widget(i)))
		{
			closeToolTab(tt);
			return;
		}
		if (auto* mt = qobject_cast<MacroTab*>(m_tabs->widget(i)))
		{
			if (mt->isDirty())
			{
				m_tabs->setCurrentIndex(i);
				statusBar()->showMessage(T(m_backend, "macro.close.dirty"), 8000);
				return;
			}
			m_tabs->removeTab(i);
			mt->deleteLater();
			return;
		}
		if (auto* ts = qobject_cast<TerminalSettingsTab*>(m_tabs->widget(i)))
		{
			m_tabs->removeTab(i);
			ts->deleteLater();
			return;
		}
		if (auto* ed = qobject_cast<PropertiesTab*>(m_tabs->widget(i)))
		{
			m_tabs->setCurrentWidget(ed);
			ed->requestClose([this, ed] {
				m_tabs->removeTab(m_tabs->indexOf(ed));
				ed->deleteLater();
			});
			return;
		}
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
		updateNotice();
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
	reloadViews();
	reloadTree();
	rebuildTabs();
}

MainWindow::~MainWindow() = default;

void MainWindow::buildMenus()
{
	QMenu* file = m_menuBar->addMenu(T(m_backend, "menu.file"));
	file->addAction(T(m_backend, "menu.file.new"), this, [this] { m_overlay->openNew(); });
	file->addAction(T(m_backend, "menu.file.open"), QKeySequence::Open, this, [this] { showUnlock(false); });
	file->addAction(T(m_backend, "menu.file.import"), this, [this] {
		const QString path = QFileDialog::getOpenFileName(this, T(m_backend, "menu.file.import"), QString(),
		                                                  QStringLiteral("Remote Desktop Manager / Royal TS / remotejoe (*.rdm *.json *.csv *.zip *.rtsz *.rtsx);;Alle Dateien (*)"));
		if (path.isEmpty())
			return;
		QString zipPassword;
		QJsonObject r = parseObject(m_backend->importPreview(path, zipPassword));
		// Verschlüsseltes ZIP (remotejoe-Export): Kennwort abfragen und erneut versuchen.
		while (r.value(QStringLiteral("error")).toString().startsWith(QLatin1String("PASSWORD_")))
		{
			bool ok = false;
			const QString text = T(m_backend, r.value(QStringLiteral("error")).toString() == QLatin1String("PASSWORD_WRONG") ? "import.zip.wrong" : "import.zip.needed");
			zipPassword = QInputDialog::getText(this, T(m_backend, "import.zip.title"), text, QLineEdit::Password, QString(), &ok);
			if (!ok)
				return;
			r = parseObject(m_backend->importPreview(path, zipPassword));
		}
		zipPassword.fill(QLatin1Char('0'));
		if (r.contains(QStringLiteral("error")))
		{
			m_bar->inform(T(m_backend, "import.failed").arg(r.value(QStringLiteral("error")).toString()));
			return;
		}
		openImport(path, r.value(QStringLiteral("report")).toObject(), r.value(QStringLiteral("summary")).toString());
	});
	file->addAction(T(m_backend, "menu.file.export"), this, [this] { openExport(); });
	file->addAction(T(m_backend, "menu.file.save"), QKeySequence::Save, this, [this] {
		// Liegt ein geänderter Eigenschaften-Tab vorn, speichert er seine Felder mit.
		if (auto* ed = qobject_cast<PropertiesTab*>(m_tabs->currentWidget()); ed && ed->isDirty())
			ed->saveNow();
		else
			saveAll();
	});
	file->addAction(T(m_backend, "menu.file.lock"), QKeySequence(QStringLiteral("Ctrl+L")), this, [this] { lockNow(false); });
	file->addSeparator();
	file->addAction(T(m_backend, "menu.file.keyring.forget"), this, [this] { forgetKeyring(); });
	file->addSeparator();
	file->addAction(T(m_backend, "menu.file.quit"), QKeySequence::Quit, this, &QWidget::close);
	QMenu* editMenu = m_menuBar->addMenu(T(m_backend, "menu.edit"));
	m_undoAction = editMenu->addAction(T(m_backend, "menu.edit.undo.none") + QStringLiteral("\tCtrl+Z"), this, [this] { undo(); });
	connect(editMenu, &QMenu::aboutToShow, this, [this] {
		const QString label = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("undo_label") } })).value(QStringLiteral("label")).toString();
		m_undoAction->setText((label.isEmpty() ? T(m_backend, "menu.edit.undo.none") : T(m_backend, "menu.edit.undo").arg(label)) + QStringLiteral("\tCtrl+Z"));
		m_undoAction->setEnabled(!label.isEmpty());
	});
	editMenu->addSeparator();
	editMenu->addAction(T(m_backend, "menu.edit.term.copy"), this, [] { if (auto* t = TerminalView::active()) t->copySelection(); });
	editMenu->addAction(T(m_backend, "menu.edit.term.paste"), this, [] { if (auto* t = TerminalView::active()) t->pasteClipboard(); });
	editMenu->addAction(T(m_backend, "menu.edit.properties") + QStringLiteral("\tF4"), this, [this] {
		if (!currentId().isEmpty())
			openProperties(currentId());
	});
	QMenu* add = editMenu->addMenu(T(m_backend, "menu.edit.add"));
	connect(add, &QMenu::aboutToShow, this, [this, add] {
		add->clear();
		buildAddMenu(add, currentParentId());
	});
	editMenu->addAction(T(m_backend, "ctx.rename") + QStringLiteral("\tF2"), this, [this] { renameSelected(); });
	editMenu->addAction(T(m_backend, "ctx.duplicate") + QStringLiteral("\tCtrl+D"), this, [this] { duplicateSelected(); });
	editMenu->addAction(T(m_backend, "ctx.move") + QStringLiteral("\tCtrl+Shift+M"), this, [this] { moveSelected(); });
	editMenu->addAction(T(m_backend, "ctx.delete") + QStringLiteral("\tEntf"), this, [this] { deleteSelected(); });
	editMenu->addAction(T(m_backend, "ctx.favorite.on") + QStringLiteral("\tCtrl+B"), this, [this] { toggleFavorite(); });
	editMenu->addSeparator();
	editMenu->addAction(T(m_backend, "menu.edit.view.new"), this, [this] { viewCommand(QStringLiteral("new")); });
	editMenu->addAction(T(m_backend, "menu.edit.view.rename"), this, [this] { viewCommand(QStringLiteral("rename")); });
	editMenu->addAction(T(m_backend, "menu.edit.view.delete"), this, [this] { viewCommand(QStringLiteral("delete")); });
	QMenu* extras = m_menuBar->addMenu(T(m_backend, "menu.extras"));
	extras->addAction(T(m_backend, "menu.extras.idle"), this, [this] { idleSettings(); });
	extras->addAction(T(m_backend, "menu.extras.clipclear"), this, [this] { clipboardClearSettings(); });
	extras->addAction(T(m_backend, "menu.extras.snmp"), this, [this] {
		for (int i = 0; i < m_tabs->count(); ++i)
			if (qobject_cast<SnmpTab*>(m_tabs->widget(i)))
			{
				m_tabs->setCurrentIndex(i);
				return;
			}
		auto* t = new SnmpTab(m_backend, m_tabs);
		connect(t, &SnmpTab::status, this, [this](const QString& m) { statusBar()->showMessage(m, 5000); });
		m_tabs->setCurrentIndex(m_tabs->addTab(t, T(m_backend, "snmp.title")));
	});
	extras->addAction(T(m_backend, "menu.extras.terminal"), this, [this] { openTerminalSettings(); });
	extras->addAction(T(m_backend, "menu.extras.appearance"), this, [this] { openAppearance(); });
	{
		QAction* kc = extras->addAction(T(m_backend, "menu.extras.keycapture"));
		kc->setCheckable(true);
		kc->setChecked(FrameView::s_keyCapture);
		connect(kc, &QAction::toggled, this, [this](bool on) {
			FrameView::s_keyCapture = on;
			if (!on)
				KeyCapture::release();
			edit({ { QStringLiteral("op"), QStringLiteral("set_settings") }, { QStringLiteral("keyboard_capture"), on } });
			statusBar()->showMessage(T(m_backend, on ? "keys.on" : "keys.off"), 6000);
		});
		connect(extras, &QMenu::aboutToShow, kc, [kc] { kc->setChecked(FrameView::s_keyCapture); });
	}
	QMenu* help = nullptr;
	QMenu* view = m_menuBar->addMenu(T(m_backend, "menu.view"));
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
	view->addAction(T(m_backend, "menu.view.term.bigger"), this, [this] { TerminalView::applyToAll(parseObject(m_backend->terminalFontStep(1))); });
	view->addAction(T(m_backend, "menu.view.term.smaller"), this, [this] { TerminalView::applyToAll(parseObject(m_backend->terminalFontStep(-1))); });
	view->addAction(T(m_backend, "menu.view.term.default"), this, [this] { TerminalView::applyToAll(parseObject(m_backend->terminalFontStep(0))); });
	view->addSeparator();
	view->addAction(T(m_backend, "menu.view.detach"), QKeySequence(QStringLiteral("Ctrl+Shift+D")), this, [this] {
		if (m_backend->detachTab(currentTab()))
			rebuildTabs();
	});
	view->addSeparator();
	view->addAction(T(m_backend, "menu.view.tree"), QKeySequence(Qt::Key_F9), this, [this] { setTreeVisible(m_treeHidden); });
	m_compactAction = view->addAction(T(m_backend, "menu.view.compact"));
	m_compactAction->setShortcut(QKeySequence(Qt::Key_F10));
	m_compactAction->setCheckable(true);
	connect(m_compactAction, &QAction::triggered, this, [this](bool on) { setCompact(on); });
	view->addAction(T(m_backend, "menu.view.reset_window"), this, [this] { resetWindow(); });
	help = m_menuBar->addMenu(T(m_backend, "menu.help"));
	help->addAction(T(m_backend, "menu.help.update"), this, [this] { openUpdateTab(); });
	help->addAction(T(m_backend, "menu.help.help"), QKeySequence::HelpContents, this, [] { QDesktopServices::openUrl(QUrl(QString::fromLatin1(kHelpUrl))); });
	help->addSeparator();
	// Das einzige Popup des Programms: „Über remotejoe“.
	help->addAction(T(m_backend, "menu.help.about"), this, [this] {
		auto* d = new AboutDialog(m_backend, m_backend->appVersion(), this);
		d->open();
	});
}

QWidget* MainWindow::aboutForScreenshot()
{
	auto* d = new AboutDialog(m_backend, m_backend->appVersion(), this);
	d->show();
	return d;
}

void MainWindow::updateForScreenshot(const QString& base)
{
	QJsonObject o;
	o.insert(QStringLiteral("op"), QStringLiteral("update_demo_source"));
	o.insert(QStringLiteral("base_url"), base);
	m_backend->edit(QString::fromUtf8(QJsonDocument(o).toJson(QJsonDocument::Compact)));
	openUpdateTab();
	if (auto* u = findChild<UpdateTab*>())
		u->check();
}

void MainWindow::openUpdateTab()
{
	if (auto* u = findChild<UpdateTab*>())
	{
		u->refresh();
		m_tabs->setCurrentWidget(u);
		return;
	}
	addToolTab(new UpdateTab(m_backend, m_tabs), T(m_backend, "update.title"));
}

void MainWindow::applyTheme(bool dark)
{
	qApp->setStyle(QStyleFactory::create(QStringLiteral("Fusion")));
	QJsonObject p = parseObject(m_backend->paletteJson());
	if (p.value(QStringLiteral("mode")).toString() != QLatin1String("system") && qEnvironmentVariableIsSet("REMOTEJOE_THEME") && dark != (QColor(p.value(QStringLiteral("base")).toString()).lightness() < 128))
	{
		// Gegenteiliges Schema erzwingen (für Bildschirmfotos).
		qputenv("REMOTEJOE_THEME", dark ? "dark" : "light");
		p = parseObject(m_backend->paletteJson());
	}
	// „System“: Farben aus der Stilpalette (hell oder dunkel und Akzentfarbe vom Desktop);
	// sonst ein Standardschema oder eine eigene Palette.
	const bool system = p.value(QStringLiteral("mode")).toString() == QLatin1String("system");
	QPalette pal = system ? qApp->style()->standardPalette() : QPalette();
	auto c = [&](const char* k) -> QColor {
		if (system)
		{
			const QString key = QString::fromLatin1(k);
			if (key == QLatin1String("window"))
				return pal.color(QPalette::Window);
			if (key == QLatin1String("text"))
				return pal.color(QPalette::WindowText);
			if (key == QLatin1String("base"))
				return pal.color(QPalette::Base);
			if (key == QLatin1String("alternate"))
				return pal.color(QPalette::AlternateBase);
			if (key == QLatin1String("muted"))
				return pal.color(QPalette::PlaceholderText);
			if (key == QLatin1String("button"))
				return pal.color(QPalette::Button);
			if (key == QLatin1String("highlight"))
				return pal.color(QPalette::Highlight);
			if (key == QLatin1String("highlight_text"))
				return pal.color(QPalette::HighlightedText);
			if (key == QLatin1String("link"))
				return pal.color(QPalette::Link);
		}
		return QColor(p.value(QString::fromLatin1(k)).toString());
	};
	if (!system)
	{
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
	}
	qApp->setPalette(pal);
	// Trennlinie der Tabs: Mischung aus Text- und Fensterfarbe, in beiden Schemata gut sichtbar.
	const QColor textColor = c("text"), windowColor = c("window");
	const QColor tabLine((textColor.red() * 55 + windowColor.red() * 45) / 100, (textColor.green() * 55 + windowColor.green() * 45) / 100,
	                     (textColor.blue() * 55 + windowColor.blue() * 45) / 100);
	qApp->setStyleSheet(QStringLiteral(
	  "QTreeView::item { min-height: 28px; padding: 2px; }"
	  // Tabs: der aktive Tab hat Akzentbalken, kräftige Schrift und den Hintergrund der Fläche darunter;
	  // inaktive Tabs sind abgesetzt (Schaltflächenfarbe, Nebentext) und durch Linien klar getrennt.
	  "QTabWidget::pane { border: 1px solid %3; border-top: 2px solid %2; top: -1px; }"
	  "QTabBar::tab { min-height: 30px; min-width: 70px; max-width: 230px; padding: 4px 16px 4px 12px; margin-right: 2px; background: %4; color: %5; border: 1px solid %3; border-bottom: none; "
	  "border-top-left-radius: 4px; border-top-right-radius: 4px; }"
	  "QTabBar::close-button { subcontrol-position: right; margin-right: 8px; }"
  "QTabBar::tab:selected { background: %6; color: %7; border-top: 3px solid %2; padding-top: 3px; }"
	  "QTabBar::tab:!selected { margin-top: 3px; }"
	  "QTabBar[compact=\"true\"]::tab { min-height: 20px; padding: 2px 14px 2px 10px; }"
	  "QTabBar[compact=\"true\"]::tab:selected { padding-top: 1px; }"
	  "QTabBar::tab:!selected:hover { background: %8; color: %7; }"
	  "QLabel#paneHeader { font-weight: bold; padding: 1px 4px; }"
	  "QLabel#paneNotice { color: %1; padding: 4px; }"
	  "QPushButton { min-height: 30px; padding: 2px 12px; }"
	  "QPushButton#paneBtn { min-height: 16px; padding: 0 8px; }"
	  "QStatusBar { padding: 0; margin: 0; } QStatusBar::item { border: none; }"
	  "QStatusBar QLabel { padding: 0; margin: 0; }"
	  "QStatusBar QPushButton { min-height: 12px; padding: 0 6px; }"
	  "QToolButton#statusTreeButton { padding: 0 6px; margin: 0; min-height: 0; }"
	  "QLabel#statusNotice { color: %1; }"
	  "QLabel#statusNotice[severe=\"true\"] { color: #e53935; font-weight: bold; }"
	  "QLabel#fieldError, QLabel#dirtyLabel { color: %1; font-weight: bold; }"
	  "QFrame#actionBar { border: 2px solid %2; border-radius: 4px; }"
	  "QFrame#securityBar { border: 2px solid %2; border-radius: 4px; }"
	  "QFrame#securityBar[changed=\"true\"] { border: 3px solid %1; }"
	  "QFrame#securityBar[changed=\"true\"] QLabel#securityTitle { color: %1; font-weight: bold; }"
	  "QLabel#securityTitle { font-weight: bold; }"
	  "QLabel#heading { font-weight: bold; padding: 4px 0; }"
	  "QFrame#unlockBox { border: 2px solid %2; border-radius: 6px; padding: 12px; }"
	  "*[rjDirty=\"true\"] { border: 2px solid %2; }"
	  "QRadioButton[rjDirty=\"true\"], QCheckBox[rjDirty=\"true\"] { border: none; font-weight: bold; color: %2; }")
	                      .arg(c("warning").name(), c("highlight").name())
	                      .arg(tabLine.name(), c("button").name(), c("muted").name(), c("base").name(), c("text").name(), c("alternate").name()));
}

void MainWindow::reloadTree()
{
	// Aufgeklappte Ordner, Auswahl und Bildlauf merken, damit eine Änderung den Baum nicht zuklappt.
	// Während einer Suche klappt alles auf; dann bleibt der zuletzt bekannte Zustand ohne Suche erhalten.
	const bool searching = !m_search->text().isEmpty();
	if (!searching && m_treeLoaded)
	{
		m_expandedIds.clear();
		std::function<void(const QModelIndex&)> collect = [&](const QModelIndex& parent) {
			for (int r = 0; r < m_model->rowCount(parent); ++r)
			{
				const QModelIndex i = m_model->index(r, 0, parent);
				if (m_tree->isExpanded(i))
				{
					const QString id = i.data(RoleId).toString();
					if (!id.isEmpty())
						m_expandedIds.insert(id);
				}
				collect(i);
			}
		};
		collect(QModelIndex());
	}
	const QString current = m_tree->currentIndex().isValid() ? m_tree->currentIndex().data(RoleId).toString() : QString();
	const int scroll = m_tree->verticalScrollBar()->value();

	const QString json = m_backend->treeJson(m_search->text());
	m_model->clear();
	addNodes(m_model->invisibleRootItem(), QJsonDocument::fromJson(json.toUtf8()).array());
	if (searching)
		m_tree->expandAll();
	else if (m_treeLoaded)
	{
		std::function<void(const QModelIndex&)> restore = [&](const QModelIndex& parent) {
			for (int r = 0; r < m_model->rowCount(parent); ++r)
			{
				const QModelIndex i = m_model->index(r, 0, parent);
				if (m_expandedIds.contains(i.data(RoleId).toString()))
					m_tree->expand(i);
				restore(i);
			}
		};
		restore(QModelIndex());
	}
	else
	{
		m_tree->expandToDepth(0);
		// Erster Start: den ersten Ordner mit Zielen aufklappen.
		if (m_model->rowCount() > 0)
			m_tree->expand(m_model->index(0, 0, m_model->index(0, 0)));
	}
	m_treeLoaded = true;
	addMacroShelf();
	if (!current.isEmpty())
	{
		std::function<QModelIndex(const QModelIndex&)> find = [&](const QModelIndex& parent) -> QModelIndex {
			for (int r = 0; r < m_model->rowCount(parent); ++r)
			{
				const QModelIndex i = m_model->index(r, 0, parent);
				if (i.data(RoleId).toString() == current)
					return i;
				if (const QModelIndex f = find(i); f.isValid())
					return f;
			}
			return QModelIndex();
		};
		if (const QModelIndex i = find(QModelIndex()); i.isValid())
			m_tree->setCurrentIndex(i);
	}
	m_tree->verticalScrollBar()->setValue(scroll);
	updateEmptyHint();
}

void MainWindow::addMacroShelf()
{
	// Das Fach erscheint nur in der Ansicht „Alle Ziele“ und ohne Suchtext.
	if (!m_search->text().isEmpty() || !m_viewBox->currentData().toString().isEmpty())
		return;
	const QJsonArray macros = QJsonDocument::fromJson(edit({ { QStringLiteral("op"), QStringLiteral("macro_list") } }).toUtf8()).array();
	auto* shelf = new QStandardItem(protocolIcon(QString(), QString(), true), T(m_backend, "macro.shelf"));
	shelf->setData(1, RoleMacro);
	shelf->setEditable(false);
	shelf->setDragEnabled(false);
	shelf->setDropEnabled(false);
	for (const QJsonValue& v : macros)
	{
		const QJsonObject m = v.toObject();
		auto* it = new QStandardItem(protocolIcon(QStringLiteral("macro"), QString(), false), m.value(QStringLiteral("name")).toString());
		it->setData(2, RoleMacro);
		it->setData(m.value(QStringLiteral("id")).toString(), RoleId);
		it->setEditable(false);
		it->setDragEnabled(false);
		it->setDropEnabled(false);
		it->setToolTip(T(m_backend, "macro.links").arg(m.value(QStringLiteral("links")).toInt()));
		shelf->appendRow(it);
	}
	m_model->invisibleRootItem()->appendRow(shelf);
	if (m_macroShelfOpen)
		m_tree->expand(shelf->index());
}

void MainWindow::updateEmptyHint()
{
	if (!m_emptyHint)
		return;
	m_emptyHint->setGeometry(m_tree->viewport()->rect());
	int rows = m_model->rowCount();
	for (int r = 0; r < m_model->rowCount(); ++r)
		if (m_model->index(r, 0).data(RoleMacro).toInt() == 1)
			--rows;
	// Zustand des Hinweises: gesperrt (keine Datenbank entsperrt) oder leer (entsperrt, keine Ziele).
	const bool open = m_backend->isVaultOpen();
	m_emptyLabel->setText(T(m_backend, open ? "tree.empty" : "tree.locked"));
	m_emptyButton->setText(T(m_backend, open ? "tree.empty.add" : "tree.locked.open"));
	m_emptyHint->setVisible((rows <= 0 || !open) && m_search->text().isEmpty());
	m_emptyHint->raise();
}

int MainWindow::treeRowCount() const
{
	int n = 0;
	std::function<void(const QModelIndex&)> walk = [&](const QModelIndex& parent) {
		for (int r = 0; r < m_model->rowCount(parent); ++r)
		{
			const QModelIndex i = m_model->index(r, 0, parent);
			// Platzhalter und Gäste von Hypervisoren zählen nicht als Index-Einträge.
			if (i.data(RolePlaceholder).toBool() || i.data(RoleGuest).isValid() || i.data(RoleMacro).toInt())
				continue;
			++n;
			walk(i);
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
	// Mehrere markierte Ziele: alle gestaffelt öffnen (Ordner in der Markierung zählen nicht).
	QStringList ids;
	for (const QModelIndex& i : m_tree->selectionModel()->selectedIndexes())
		if (!i.data(RoleFolder).toBool() && !i.data(RoleGuest).isValid() && !i.data(RolePlaceholder).toBool() && !i.data(RoleMacro).toInt())
			ids << i.data(RoleId).toString();
	ids.removeDuplicates();
	if (ids.size() > 1 && credential.isEmpty())
	{
		connectMany({ { QStringLiteral("ids"), QJsonArray::fromStringList(ids) } });
		return;
	}
	connectCurrent(credential, remember);
}

bool MainWindow::confirmManyTargets(int n)
{
	if (n <= kConnectWithoutAsking)
		return true;
	QMessageBox box(QMessageBox::Question, T(m_backend, "connect.many.title"), T(m_backend, "connect.many.text").arg(n), QMessageBox::NoButton, this);
	auto* ok = box.addButton(T(m_backend, "connect.many.ok"), QMessageBox::AcceptRole);
	box.setDefaultButton(box.addButton(T(m_backend, "connect.many.cancel"), QMessageBox::RejectRole));
	box.exec();
	return box.clickedButton() == ok;
}

void MainWindow::connectMany(const QJsonObject& which)
{
	QJsonObject count = which;
	count.insert(QStringLiteral("op"), QStringLiteral("connect_all"));
	count.insert(QStringLiteral("count_only"), true);
	if (!confirmManyTargets(parseObject(edit(count)).value(QStringLiteral("count")).toInt()))
		return;
	const QSize px = contentSize(false);
	QJsonObject cmd = which;
	cmd.insert(QStringLiteral("op"), QStringLiteral("connect_all"));
	cmd.insert(QStringLiteral("mode"), QStringLiteral("connect"));
	cmd.insert(QStringLiteral("width"), px.width());
	cmd.insert(QStringLiteral("height"), px.height());
	const QJsonObject r = parseObject(edit(cmd));
	if (r.contains(QStringLiteral("error")))
		statusBar()->showMessage(r.value(QStringLiteral("error")).toString(), 8000);
	else
		statusBar()->showMessage(T(m_backend, "status.connect_all").arg(r.value(QStringLiteral("count")).toInt()).arg(4), 8000);
	rebuildTabs();
}

void MainWindow::connectCurrent(const QString& credential, bool remember)
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
	if (r.contains(QStringLiteral("pending")))
	{
		// Tunnel oder Konsole wird im Hintergrund aufgebaut.
		m_jobTargets.insert(qulonglong(r.value(QStringLiteral("pending")).toInteger()), id);
		updateJobs();
		return;
	}
	if (r.contains(QStringLiteral("certificate")))
	{
		statusBar()->showMessage(T(m_backend, "guest.cert.unknown").arg(r.value(QStringLiteral("certificate")).toObject().value(QStringLiteral("host")).toString()), 8000);
		return;
	}
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
		chooseCredential(promptTab(id), r.value(QStringLiteral("ambiguous")).toArray());
		return;
	}
	if (r.contains(QStringLiteral("hostkey")))
	{
		// Unbekannter Host-Schlüssel eines Sprungservers: nie still annehmen.
		const QJsonObject h = r.value(QStringLiteral("hostkey")).toObject();
		ConnectPromptTab* tab = promptTab(id);
		QJsonObject q{ { QStringLiteral("text"), h.value(QStringLiteral("text")) },
			           { QStringLiteral("detail"), h.value(QStringLiteral("detail")) },
			           { QStringLiteral("timeout_secs"), m_backend->questionTimeout() },
			           { QStringLiteral("info"), QJsonObject{ { QStringLiteral("kind"), QStringLiteral("hostkey") },
			                                                  { QStringLiteral("host"), h.value(QStringLiteral("host")) },
			                                                  { QStringLiteral("port"), h.value(QStringLiteral("port")) },
			                                                  { QStringLiteral("key_type"), h.value(QStringLiteral("key_type")) },
			                                                  { QStringLiteral("fingerprint"), h.value(QStringLiteral("fingerprint")) },
			                                                  { QStringLiteral("previous"), h.value(QStringLiteral("previous")) } } } };
		QPointer<ConnectPromptTab> guard(tab);
		tab->security()->showQuestion(0, q, [this, id, h, guard](bool accept) {
			if (guard)
				closeToolTab(guard);
			if (accept &&
			    m_backend->acceptHostKey(h.value(QStringLiteral("host")).toString(), std::uint32_t(h.value(QStringLiteral("port")).toInt()),
			                             h.value(QStringLiteral("key")).toString()))
			{
				const QSize px = contentSize(false);
				handleConnectResult(id, m_backend->connectTarget(id, QString(), false, px.width(), px.height()));
			}
		});
		return;
	}
	if (r.contains(QStringLiteral("missing")))
	{
		const QJsonObject m = r.value(QStringLiteral("missing")).toObject();
		QString title = m.value(QStringLiteral("title")).toString();
		m_bar->inform(T(m_backend, "missing.credential.text")
		                .arg(T(m_backend, "missing.credential"), title.isEmpty() ? m.value(QStringLiteral("name")).toString() : title,
		                     m.value(QStringLiteral("reason")).toString()));
		return;
	}
	m_bar->inform(r.value(QStringLiteral("error")).toString());
}

ConnectPromptTab* MainWindow::promptTab(const QString& target)
{
	for (auto* t : findChildren<ConnectPromptTab*>())
		if (t->targetId() == target)
		{
			m_tabs->setCurrentWidget(t);
			return t;
		}
	QString name = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("editor") }, { QStringLiteral("id"), target } })).value(QStringLiteral("name")).toString();
	if (name.isEmpty())
		name = target;
	auto* t = new ConnectPromptTab(m_backend, target, name, m_tabs);
	addToolTab(t, name);
	return t;
}

void MainWindow::chooseCredential(ConnectPromptTab* tab, const QJsonArray& choices)
{
	QList<QPair<QString, QString>> items;
	for (const QJsonValue& c : choices)
	{
		const QJsonObject o = c.toObject();
		QString label = o.value(QStringLiteral("name")).toString();
		const QString title = o.value(QStringLiteral("title")).toString();
		if (!title.isEmpty())
			label += QStringLiteral(" (") + title + QLatin1Char(')');
		items.append({ label, o.value(QStringLiteral("name")).toString() });
	}
	const QString id = tab->targetId();
	QPointer<ConnectPromptTab> guard(tab);
	tab->setMessage(T(m_backend, "dialog.credential.text"));
	tab->bar()->choose(T(m_backend, "dialog.credential.title"), items, nullptr, T(m_backend, "dialog.credential.remember"),
	                   [this, id, guard](const QString& name, bool remember) {
		                   if (guard)
			                   closeToolTab(guard);
		                   const QSize px = contentSize(false);
		                   handleConnectResult(id, m_backend->connectTarget(id, name, remember, px.width(), px.height()));
	                   });
	connect(tab->bar(), &ActionBar::cancelled, tab, [this, guard] {
		if (guard)
			closeToolTab(guard);
	});
}

void MainWindow::addToolTab(ToolTab* t, const QString& title)
{
	connect(t, &ToolTab::indexChanged, this, [this] {
		reloadTree();
		reloadViews();
	});
	connect(t, &ToolTab::status, this, [this](const QString& m) { statusBar()->showMessage(m, 8000); });
	connect(t, &ToolTab::closeRequested, this, [this, t] { closeToolTab(t); });
	m_tabs->setCurrentIndex(m_tabs->addTab(t, title));
}

void MainWindow::closeToolTab(ToolTab* t)
{
	const int i = m_tabs->indexOf(t);
	if (i < 0)
		return;
	m_tabs->removeTab(i);
	t->wipeSecrets();
	t->deleteLater();
}

void MainWindow::openImport(const QString& path, const QJsonObject& report, const QString& summary)
{
	addToolTab(new ImportTab(m_backend, path, report, summary, m_tabs), T(m_backend, "import.title"));
}

void MainWindow::showProperties(const QString& id)
{
	openProperties(id);
}

void MainWindow::expandFirstHypervisor()
{
	std::function<bool(const QModelIndex&)> walk = [&](const QModelIndex& parent) {
		for (int r = 0; r < m_model->rowCount(parent); ++r)
		{
			const QModelIndex i = m_model->index(r, 0, parent);
			if (i.data(RoleProtocol).toString() == QStringLiteral("proxmox"))
			{
				m_tree->expand(i);
				m_tree->setCurrentIndex(m_model->index(0, 0, i));
				return true;
			}
			if (walk(i))
				return true;
		}
		return false;
	};
	walk(QModelIndex());
}

void MainWindow::loadGuests(const QModelIndex& index)
{
	QStandardItem* parent = m_model->itemFromIndex(index);
	if (!parent)
		return;
	const QString target = index.data(RoleId).toString();
	const QJsonObject r = parseObject(m_backend->guestsJson(target));
	if (r.contains(QStringLiteral("pending")))
	{
		// Abruf läuft im Hintergrund; das Ergebnis kommt als Ereignis.
		if (parent->rowCount() == 1 && parent->child(0)->data(RolePlaceholder).toBool())
			parent->child(0)->setText(QStringLiteral("Wird geladen …"));
		updateJobs();
		return;
	}
	applyGuests(target, r);
}

void MainWindow::applyGuests(const QString& target, QJsonObject r)
{
	QStandardItem* parent = nullptr;
	std::function<void(const QModelIndex&)> find = [&](const QModelIndex& p) {
		for (int row = 0; row < m_model->rowCount(p) && !parent; ++row)
		{
			const QModelIndex i = m_model->index(row, 0, p);
			if (i.data(RoleId).toString() == target && !i.data(RoleGuest).isValid())
				parent = m_model->itemFromIndex(i);
			else
				find(i);
		}
	};
	find(QModelIndex());
	if (!parent)
		return;
	const QString proto = parent->data(RoleProtocol).toString();
	if (r.contains(QStringLiteral("certificate")))
	{
		// Unbekanntes oder geändertes Zertifikat: nie still annehmen.
		const QJsonObject c = r.value(QStringLiteral("certificate")).toObject();
		const bool changed = !c.value(QStringLiteral("old")).isNull() && c.contains(QStringLiteral("old"));
		ConnectPromptTab* tab = promptTab(target);
		QJsonObject q{ { QStringLiteral("text"), T(m_backend, changed ? "guest.cert.changed" : "guest.cert.unknown").arg(c.value(QStringLiteral("host")).toString()) },
			           { QStringLiteral("timeout_secs"), m_backend->questionTimeout() },
			           { QStringLiteral("info"), QJsonObject{ { QStringLiteral("kind"), QStringLiteral("certificate") },
			                                                  { QStringLiteral("host"), c.value(QStringLiteral("host")) },
			                                                  { QStringLiteral("key_type"), QStringLiteral("X.509") },
			                                                  { QStringLiteral("fingerprint"), c.value(QStringLiteral("fingerprint")) },
			                                                  { QStringLiteral("previous"), c.value(QStringLiteral("old")) } } } };
		QPointer<ConnectPromptTab> guard(tab);
		const QPersistentModelIndex pidx(parent->index());
		tab->security()->showQuestion(0, q, [this, c, guard, pidx](bool accept) {
			if (guard)
				closeToolTab(guard);
			if (accept && pidx.isValid() &&
			    m_backend->acceptProxmoxCertificate(c.value(QStringLiteral("host")).toString(), c.value(QStringLiteral("fingerprint")).toString()))
				loadGuests(pidx);
		});
		return;
	}
	parent->removeRows(0, parent->rowCount());
	if (r.contains(QStringLiteral("error")))
	{
		auto* e = new QStandardItem(T(m_backend, "guest.error").arg(r.value(QStringLiteral("error")).toString()));
		e->setEditable(false);
		e->setData(true, RolePlaceholder);
		e->setData(true, RoleFolder);
		parent->appendRow(e);
		return;
	}
	const QString consoleProto = proto == QStringLiteral("proxmox") ? QStringLiteral("vnc") : QStringLiteral("rdp");
	for (const QJsonValue& v : r.value(QStringLiteral("guests")).toArray())
	{
		const QJsonObject g = v.toObject();
		const QString state = g.value(QStringLiteral("state")).toString();
		auto* item = new QStandardItem(g.value(QStringLiteral("name")).toString());
		item->setEditable(false);
		item->setData(g.value(QStringLiteral("id")).toString(), RoleGuest);
		item->setData(target, RoleGuestTarget);
		item->setData(g.value(QStringLiteral("console_host")).toString(), RoleConsoleHost);
		item->setData(true, RoleFolder);
		item->setIcon(protocolIcon(consoleProto, state == QStringLiteral("running") ? QStringLiteral("green") : QStringLiteral("grey"), false));
		item->setToolTip(g.value(QStringLiteral("detail")).toString() + QStringLiteral("\n") + state);
		parent->appendRow(item);
	}
}

void MainWindow::openGuestConsole(const QModelIndex& i)
{
	const QSize px = contentSize(false);
	const QString target = i.data(RoleGuestTarget).toString();
	handleConnectResult(target, m_backend->guestConsole(target, i.data(RoleGuest).toString(), i.data(Qt::DisplayRole).toString(),
	                                                    i.data(RoleConsoleHost).toString(), px.width(), px.height()));
}

void MainWindow::guestAction(const QModelIndex& i, const QString& action, bool confirmed)
{
	if (action == QStringLiteral("stop") && !confirmed)
	{
		const QPersistentModelIndex pi(i);
		m_bar->ask(T(m_backend, "guest.stop.confirm").arg(i.data(Qt::DisplayRole).toString()), { T(m_backend, "guest.stop.button") }, [this, pi, action](int b) {
			if (b == 0 && pi.isValid())
				guestAction(pi, action, true);
		});
		return;
	}
	const QJsonObject r = parseObject(m_backend->guestAction(i.data(RoleGuestTarget).toString(), i.data(RoleGuest).toString(), action));
	if (r.contains(QStringLiteral("pending")))
		updateJobs();
	else if (r.contains(QStringLiteral("error")))
		m_bar->inform(r.value(QStringLiteral("error")).toString());
	else
		statusBar()->showMessage(T(m_backend, "guest.action.sent"), 4000);
}

/// Benutzername oder Kennwort eines Ziels in die Zwischenablage; ein Kennwort wird nach der
/// eingestellten Frist wieder gelöscht, aber nur, wenn noch derselbe Inhalt darin steht.
void MainWindow::copyCredential(const QString& id, const QString& kind)
{
	const QJsonObject r = parseObject(m_backend->credentialCopy(id, kind));
	if (r.contains(QStringLiteral("error")))
	{
		statusBar()->showMessage(r.value(QStringLiteral("error")).toString(), 8000);
		return;
	}
	const QString text = r.value(QStringLiteral("text")).toString();
	const bool password = kind == QLatin1String("password");
	auto* md = new QMimeData;
	md->setText(text);
	if (password)
	{
		md->setData(QStringLiteral("x-kde-passwordManagerHint"), "secret");
#ifdef Q_OS_WIN
		// Windows: Das Kennwort soll weder im Zwischenablageverlauf (Win+V) noch in der Cloud-Zwischenablage
		// und nicht in Überwachungsprogrammen landen. Qt reicht diese Formate unter ihrem Windows-Namen durch.
		const QByteArray zero(4, '\0'); // DWORD 0
		md->setData(QStringLiteral("application/x-qt-windows-mime;value=\"ExcludeClipboardContentFromMonitorProcessing\""), zero);
		md->setData(QStringLiteral("application/x-qt-windows-mime;value=\"CanIncludeInClipboardHistory\""), zero);
		md->setData(QStringLiteral("application/x-qt-windows-mime;value=\"CanUploadToCloudClipboard\""), zero);
#endif
	}
	QGuiApplication::clipboard()->setMimeData(md);
	const int secs = r.value(QStringLiteral("clear_secs")).toInt(30);
	if (!password)
	{
		statusBar()->showMessage(T(m_backend, "ctx.copy.user.done"), 5000);
		return;
	}
	if (secs <= 0)
	{
		statusBar()->showMessage(T(m_backend, "ctx.copy.password.keep"), 8000);
		return;
	}
	m_clipHash = QCryptographicHash::hash(text.toUtf8(), QCryptographicHash::Sha256);
	if (!m_clipTimer)
	{
		m_clipTimer = new QTimer(this);
		m_clipTimer->setSingleShot(true);
		connect(m_clipTimer, &QTimer::timeout, this, [this] {
			auto* cb = QGuiApplication::clipboard();
			if (QCryptographicHash::hash(cb->text().toUtf8(), QCryptographicHash::Sha256) == m_clipHash)
				cb->clear();
			m_clipHash.clear();
		});
	}
	m_clipTimer->start(secs * 1000);
	statusBar()->showMessage(T(m_backend, "ctx.copy.password.done").arg(secs), 8000);
}

void MainWindow::clipboardClearSettings()
{
	bool ok = false;
	const int v = QInputDialog::getInt(this, T(m_backend, "clip.clear.title"), T(m_backend, "clip.clear.text"),
	                                   m_backend->clipboardClearSecs(), 0, 3600, 5, &ok);
	if (ok)
		m_backend->setClipboardClearSecs(v);
}

void MainWindow::treeContextMenu(const QPoint& pos)
{
	const QModelIndex i = m_tree->indexAt(pos);
	if (!i.isValid())
	{
		// Freie Fläche (auch bei leerem Baum): Hinzufügen auf oberster Ebene anbieten.
		QMenu empty(this);
		buildAddMenu(&empty, QString());
		empty.exec(m_tree->viewport()->mapToGlobal(pos));
		return;
	}
	if (i.data(RoleMacro).toInt())
	{
		m_tree->setCurrentIndex(i);
		macroContextMenu(i, m_tree->viewport()->mapToGlobal(pos));
		return;
	}
	if (!m_tree->selectionModel()->isSelected(i))
		m_tree->setCurrentIndex(i);
	const QString id = i.data(RoleId).toString();
	QMenu menu(this);
	if (i.data(RoleGuest).isValid())
	{
		const QPersistentModelIndex pi(i);
		menu.addAction(T(m_backend, "ctx.guest.console"), this, [this, pi] { openGuestConsole(pi); });
		menu.addSeparator();
		menu.addAction(T(m_backend, "ctx.guest.start"), this, [this, pi] { guestAction(pi, QStringLiteral("start")); });
		menu.addAction(T(m_backend, "ctx.guest.shutdown"), this, [this, pi] { guestAction(pi, QStringLiteral("shutdown")); });
		menu.addAction(T(m_backend, "ctx.guest.reboot"), this, [this, pi] { guestAction(pi, QStringLiteral("reboot")); });
		menu.addAction(T(m_backend, "ctx.guest.stop"), this, [this, pi] { guestAction(pi, QStringLiteral("stop")); });
		menu.exec(m_tree->viewport()->mapToGlobal(pos));
		return;
	}
	const QString proto = i.data(RoleProtocol).toString();
	if (proto == QStringLiteral("proxmox") || proto == QStringLiteral("hyperv"))
	{
		const QPersistentModelIndex pi(i);
		menu.addAction(T(m_backend, "ctx.guest.refresh"), this, [this, pi] {
			loadGuests(pi);
			m_tree->expand(pi);
		});
	}
	if (!i.data(RoleFolder).toBool())
	{
		menu.addAction(T(m_backend, "ctx.connect"), this, [this] { connectSelected(); });
		menu.addAction(T(m_backend, "ctx.tile"), this, [this] {
			const auto tab = currentTab();
			connectCurrent();
			const auto s = currentSession();
			if (tab && s && m_backend->tileSession(s, tab))
				rebuildTabs();
		});
	}
	if (!i.data(RoleFolder).toBool())
	{
		menu.addSeparator();
		menu.addAction(T(m_backend, "ctx.copy.user"), this, [this, id] { copyCredential(id, QStringLiteral("user")); });
		menu.addAction(T(m_backend, "ctx.copy.password"), this, [this, id] { copyCredential(id, QStringLiteral("password")); });
		menu.addSeparator();
	}
	const bool folder = i.data(RoleFolder).toBool();
	QMenu* addM = menu.addMenu(T(m_backend, "ctx.add"));
	buildAddMenu(addM, folder ? id : currentParentId());
	menu.addAction(T(m_backend, "ctx.rename") + QStringLiteral("\tF2"), this, [this] { renameSelected(); });
	menu.addAction(T(m_backend, "ctx.duplicate") + QStringLiteral("\tCtrl+D"), this, [this] { duplicateSelected(); });
	menu.addAction(T(m_backend, "ctx.move") + QStringLiteral("\tCtrl+Shift+M"), this, [this] { moveSelected(); });
	menu.addAction(T(m_backend, "ctx.delete") + QStringLiteral("\tEntf"), this, [this] { deleteSelected(); });
	const bool fav = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("editor") }, { QStringLiteral("id"), id } })).value(QStringLiteral("favorite")).toBool();
	menu.addAction(T(m_backend, fav ? "ctx.favorite.off" : "ctx.favorite.on") + QStringLiteral("\tCtrl+B"), this, [this] { toggleFavorite(); });
	QMenu* viewsM = menu.addMenu(T(m_backend, "ctx.view.add"));
	for (const QJsonValue& v : QJsonDocument::fromJson(edit({ { QStringLiteral("op"), QStringLiteral("views") } }).toUtf8()).array())
	{
		const QString vid = v.toObject().value(QStringLiteral("id")).toString();
		viewsM->addAction(v.toObject().value(QStringLiteral("name")).toString(), this, [this, vid] {
			showEditResult(parseObject(edit({ { QStringLiteral("op"), QStringLiteral("view_add") }, { QStringLiteral("view"), vid }, { QStringLiteral("ids"), QJsonArray::fromStringList(selectedIds()) } })));
			reloadViews();
		});
	}
	viewsM->addSeparator();
	viewsM->addAction(T(m_backend, "ctx.view.new"), this, [this] { viewCommand(QStringLiteral("new_with")); });
	const QString curView = m_viewBox->currentData().toString();
	if (!curView.isEmpty() && curView != QLatin1String("favorites"))
		menu.addAction(T(m_backend, "ctx.view.remove"), this, [this, curView] {
			showEditResult(parseObject(edit({ { QStringLiteral("op"), QStringLiteral("view_remove") }, { QStringLiteral("view"), curView }, { QStringLiteral("ids"), QJsonArray::fromStringList(selectedIds()) } })));
		});
	if (folder)
	{
		menu.addSeparator();
		menu.addAction(T(m_backend, "ctx.all.connect"), this, [this] { connectAll(QStringLiteral("connect")); });
		menu.addAction(T(m_backend, "ctx.all.reconnect"), this, [this] { connectAll(QStringLiteral("reconnect")); });
		menu.addAction(T(m_backend, "ctx.all.disconnect"), this, [this] { connectAll(QStringLiteral("disconnect")); });
	}
	menu.addSeparator();
	menu.addAction(T(m_backend, "ctx.certcheck"), this, [this] {
		// Ein laufender Prüf-Tab wird ersetzt, damit Ergebnisse eindeutig zugeordnet bleiben.
		if (auto* old = findChild<CertCheckTab*>())
			closeToolTab(old);
		addToolTab(new CertCheckTab(m_backend, selectedIds(), m_tabs), T(m_backend, "certcheck.title"));
	});
	QMenu* tasks = menu.addMenu(T(m_backend, "ctx.tasks"));
	tasks->addAction(T(m_backend, "ctx.tasks.updates"), this, [this] {
		const QStringList ids = selectedTargets(m_tree, m_model);
		if (!ids.isEmpty())
			openPatchday(ids);
	});
	menu.addAction(T(m_backend, "ctx.properties") + QStringLiteral("\tF4"), this, [this, id] { openProperties(id); });
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

void MainWindow::showUnlock(bool locked)
{
	m_overlay->open(m_backend->kdbxPath(), locked);
}

void MainWindow::showNewVault()
{
	m_overlay->openNew();
}

void MainWindow::openMacroTab(const QString& id)
{
	for (int i = 0; i < m_tabs->count(); ++i)
		if (auto* mt = qobject_cast<MacroTab*>(m_tabs->widget(i)); mt && mt->macroId() == id)
		{
			m_tabs->setCurrentIndex(i);
			return;
		}
	auto* t = new MacroTab(m_backend, id, m_tabs);
	const int idx = m_tabs->addTab(t, t->title());
	m_tabs->setCurrentIndex(idx);
	connect(t, &MacroTab::titleChanged, this, [this, t] {
		const int i = m_tabs->indexOf(t);
		if (i >= 0)
			m_tabs->setTabText(i, t->title());
	});
	connect(t, &MacroTab::indexChanged, this, [this] { reloadTree(); });
	auto closeIt = [this, t] {
		const int i = m_tabs->indexOf(t);
		if (i >= 0)
		{
			m_tabs->removeTab(i);
			t->deleteLater();
		}
		reloadTree();
	};
	connect(t, &MacroTab::saved, this, [this, closeIt] {
		statusBar()->showMessage(T(m_backend, "macro.saved"), 5000);
		closeIt();
	});
	connect(t, &MacroTab::discarded, this, closeIt);
	reloadTree();
}

void MainWindow::deleteMacro(const QString& id, const QString& name)
{
	m_bar->ask(T(m_backend, "macro.delete.ask").arg(name), { T(m_backend, "bar.delete.button") }, [this, id](int i) {
		if (i != 0)
			return;
		for (auto* mt : findChildren<MacroTab*>())
			if (mt->macroId() == id)
			{
				m_tabs->removeTab(m_tabs->indexOf(mt));
				mt->deleteLater();
			}
		showEditResult(parseObject(edit({ { QStringLiteral("op"), QStringLiteral("macro_delete") }, { QStringLiteral("id"), id } })));
		reloadTree();
		m_tree->setFocus();
	});
}

void MainWindow::macroContextMenu(const QModelIndex& i, const QPoint& globalPos)
{
	QMenu menu(this);
	if (i.data(RoleMacro).toInt() == 1)
	{
		menu.addAction(T(m_backend, "macro.shelf.new"), this, [this] {
			const QJsonObject r = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("macro_new") } }));
			if (r.contains(QStringLiteral("macro")))
			{
				m_macroShelfOpen = true;
				openMacroTab(r.value(QStringLiteral("macro")).toString());
			}
		});
	}
	else
	{
		const QString id = i.data(RoleId).toString();
		const QString name = i.data(Qt::DisplayRole).toString();
		menu.addAction(T(m_backend, "macro.edit"), this, [this, id] { openMacroTab(id); });
		menu.addAction(T(m_backend, "macro.duplicate"), this, [this, id] {
			const QJsonObject r = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("macro_duplicate") }, { QStringLiteral("id"), id } }));
			reloadTree();
			if (r.contains(QStringLiteral("macro")))
				openMacroTab(r.value(QStringLiteral("macro")).toString());
		});
		menu.addAction(T(m_backend, "macro.delete") + QStringLiteral("\tEntf"), this, [this, id, name] { deleteMacro(id, name); });
	}
	menu.exec(globalPos);
}

void MainWindow::macrosForScreenshot()
{
	const QJsonObject r = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("macro_new") } }));
	const QString id = r.value(QStringLiteral("macro")).toString();
	const QJsonArray steps{
		QJsonObject{ { QStringLiteral("step"), QStringLiteral("send") }, { QStringLiteral("text"), QStringLiteral("sudo -v{key:enter}") } },
		QJsonObject{ { QStringLiteral("step"), QStringLiteral("wait") },
		             { QStringLiteral("pattern"), QJsonObject{ { QStringLiteral("text"), QStringLiteral("Kennwort") } } },
		             { QStringLiteral("errors"), QJsonArray{ QJsonObject{ { QStringLiteral("text"), QStringLiteral("Falsches Kennwort") } } } },
		             { QStringLiteral("timeout_ms"), 10000 } },
		QJsonObject{ { QStringLiteral("step"), QStringLiteral("send") }, { QStringLiteral("text"), QStringLiteral("{cred:password}{key:enter}") } },
		QJsonObject{ { QStringLiteral("step"), QStringLiteral("send") }, { QStringLiteral("text"), QStringLiteral("apt list --upgradable{key:enter}") } } };
	edit({ { QStringLiteral("op"), QStringLiteral("update_macro") }, { QStringLiteral("id"), id }, { QStringLiteral("name"), QStringLiteral("Updates prüfen") }, { QStringLiteral("steps"), steps } });
	m_macroShelfOpen = true;
	reloadTree();
	openMacroTab(id);
	m_tree->scrollToBottom();
}

void MainWindow::terminalSettingsForScreenshot(const QString& scheme)
{
	m_backend->setTerminalSettings(QStringLiteral("{\"scheme\":\"%1\"}").arg(scheme));
	openTerminalSettings();
	for (int i = 0; i < m_tabs->count(); ++i)
		if (auto* t = qobject_cast<TerminalSettingsTab*>(m_tabs->widget(i)))
			t->deleteLater(), m_tabs->removeTab(i);
	openTerminalSettings();
}

bool MainWindow::showWebTabForScreenshot()
{
	for (SessionPane* p : std::as_const(m_panes))
		if (p && p->isWeb())
			for (int i = 0; i < m_tabs->count(); ++i)
				if (m_tabs->widget(i)->isAncestorOf(p))
				{
					m_tabs->setCurrentIndex(i);
					if (p->frame())
						p->frame()->setFocus();
					return true;
				}
	return false;
}

void MainWindow::showEmptyTreeForScreenshot()
{
	m_model->clear();
	updateEmptyHint();
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
			p->setMacroOpener([this](const QString& macroId) { openMacroTab(macroId); });
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
		// Eigenschaften-Tabs bleiben erhalten, Sitzungs-Tabs werden neu aufgebaut.
		QWidget* current = target->currentWidget();
		QList<QPair<QWidget*, QString>> editors;
		while (target->count())
		{
			QWidget* w = target->widget(0);
			const QString text = target->tabText(0);
			target->removeTab(0);
			if (w->property("editor").toBool())
				editors.append({ w, text });
			else
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
			for (SessionPane* p : std::as_const(m_panes))
				if (p && content->isAncestorOf(p))
					applyPageTitle(p);
			// Symbol des Protokolls der ersten Sitzung im Tab.
			for (SessionPane* p : std::as_const(m_panes))
				if (p && content->isAncestorOf(p))
				{
					const QString proto = parseObject(m_backend->sessionInfo(p->session())).value(QStringLiteral("protocol")).toString();
					if (!proto.isEmpty())
						target->setTabIcon(i, protocolIcon(proto, QString(), false));
					break;
				}
		}
		target->setCurrentIndex(activeIndex);
		for (const auto& e : editors)
		{
			target->addTab(e.first, e.second);
			if (e.first == current)
				target->setCurrentWidget(e.first);
		}
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
			configureTabBar(tw);
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

void MainWindow::applyAppearance()
{
	const QJsonObject a = parseObject(m_backend->appearanceJson());
	FrameView::setFocusFrame(a.value(QStringLiteral("focus_frame")).toBool(true));
	applyTheme(qEnvironmentVariable("REMOTEJOE_THEME") == QLatin1String("dark"));
	for (auto* w : findChildren<QWidget*>())
		if (dynamic_cast<FrameView*>(w))
			w->update();
}

void MainWindow::openExport()
{
	for (int i = 0; i < m_tabs->count(); ++i)
		if (qobject_cast<ExportTab*>(m_tabs->widget(i)))
		{
			m_tabs->setCurrentIndex(i);
			return;
		}
	auto* t = new ExportTab(m_backend, [this] { return selectedIds(); }, m_tabs);
	connect(t, &ToolTab::status, this, [this](const QString& text) { statusBar()->showMessage(text, 10000); });
	m_tabs->setCurrentIndex(m_tabs->addTab(t, T(m_backend, "export.title")));
}

void MainWindow::openAppearance()
{
	for (int i = 0; i < m_tabs->count(); ++i)
		if (qobject_cast<AppearanceTab*>(m_tabs->widget(i)))
		{
			m_tabs->setCurrentIndex(i);
			return;
		}
	auto* t = new AppearanceTab(m_backend, m_tabs);
	connect(t, &AppearanceTab::appearanceChanged, this, [this] { applyAppearance(); });
	connect(t, &ToolTab::status, this, [this](const QString& text) { statusBar()->showMessage(text, 6000); });
	m_tabs->setCurrentIndex(m_tabs->addTab(t, T(m_backend, "appearance.title")));
}

void MainWindow::openTerminalSettings()
{
	for (int i = 0; i < m_tabs->count(); ++i)
		if (qobject_cast<TerminalSettingsTab*>(m_tabs->widget(i)))
		{
			m_tabs->setCurrentIndex(i);
			return;
		}
	m_tabs->setCurrentIndex(m_tabs->addTab(new TerminalSettingsTab(m_backend, m_tabs), T(m_backend, "term.title")));
}

void MainWindow::updateConnInfo()
{
	const auto s = currentSession();
	const QJsonObject i = s ? parseObject(m_backend->sessionInfo(s)) : QJsonObject();
	if (i.isEmpty())
	{
		m_connInfo->hide();
		return;
	}
	const QColor c = palette().color(QPalette::WindowText);
	if (c != m_ciColor)
	{
		m_ciColor = c;
		m_ciTargetIcon->setPixmap(svgPixmap(kSvgTarget, c, 22));
		m_ciSinceIcon->setPixmap(svgPixmap(kSvgClock, c, 22));
		m_ciUserIcon->setPixmap(svgPixmap(kSvgUser, c, 22));
	}
	const QString proto = i.value(QStringLiteral("protocol")).toString();
	m_ciProtoIcon->setPixmap(protocolIcon(proto, QString(), false).pixmap(22, 22));
	m_ciProto->setText(protocolName(proto));
	const int port = i.value(QStringLiteral("port")).toInt();
	const QString host = i.value(QStringLiteral("host")).toString();
	m_ciTarget->setText(port > 0 ? QStringLiteral("%1:%2").arg(host).arg(port) : host);
	const QString state = i.value(QStringLiteral("state")).toString();
	if (i.value(QStringLiteral("since")).isNull() || state != QLatin1String("connected"))
		m_ciSince->setText(m_backend->text(QStringLiteral("state.") + (state.isEmpty() ? QStringLiteral("connecting") : state)));
	else
	{
		const QDateTime start = QDateTime::fromSecsSinceEpoch(qint64(i.value(QStringLiteral("since")).toDouble()));
		const qint64 sec = qMax<qint64>(0, start.secsTo(QDateTime::currentDateTime()));
		QString dur;
		if (sec < 60)
			dur = T(m_backend, "ci.dur.s").arg(sec);
		else if (sec < 3600)
			dur = T(m_backend, "ci.dur.ms").arg(sec / 60).arg(sec % 60);
		else
			dur = T(m_backend, "ci.dur.hm").arg(sec / 3600).arg((sec % 3600) / 60);
		const QString when = start.date() == QDate::currentDate() ? start.toString(QStringLiteral("HH:mm")) : start.toString(QStringLiteral("dd.MM. HH:mm"));
		m_ciSince->setText(T(m_backend, "ci.since").arg(when, dur));
	}
	const QString user = i.value(QStringLiteral("user")).toString();
	m_ciUser->setText(user.isEmpty() ? T(m_backend, "ci.nouser") : T(m_backend, "ci.user").arg(user));
	m_connInfo->show();
}

void MainWindow::updateStatus()
{
	QString state;
	const auto s = currentSession();
	if (auto* p = m_panes.value(s, nullptr))
		state = p->title() + QStringLiteral(": ") + m_backend->text(QStringLiteral("state.") + (p->state().isEmpty() ? QStringLiteral("connecting") : p->state()));
	statusBar()->showMessage(T(m_backend, "status.sessions").arg(m_panes.size()) + (state.isEmpty() ? QString() : QStringLiteral(" — ") + state));
	updateConnInfo();
}

void MainWindow::poll()
{
	handleEvents(m_backend->poll());
	for (const auto s : std::as_const(m_dirtyTerminals))
		if (auto* p = pane(s); p && p->terminal())
			p->terminal()->refresh();
	m_dirtyTerminals.clear();
}

void MainWindow::applyPageTitle(SessionPane* p)
{
	// Titel der Webseite als Tab-Titel, wenn der Tab nur diese Sitzung zeigt.
	if (p->pageTitle().isEmpty())
		return;
	QList<QTabWidget*> all = findChildren<QTabWidget*>();
	for (auto& w : m_windows)
		if (w)
			all += w->findChildren<QTabWidget*>();
	for (QTabWidget* tabs : all)
		for (int i = 0; i < tabs->count(); ++i)
			if (tabs->widget(i)->isAncestorOf(p) &&
			    std::count_if(m_panes.cbegin(), m_panes.cend(), [&](SessionPane* o) { return o && tabs->widget(i)->isAncestorOf(o); }) == 1)
			{
				tabs->setTabText(i, p->pageTitle());
				tabs->setTabToolTip(i, p->title() + QStringLiteral(" — ") + p->pageTitle());
			}
}

void MainWindow::handleEvents(const QString& json)
{
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
				updateNotice();
				updateStatus();
			}
			else if (t == QLatin1String("shared_frame"))
			{
				const auto fd = std::int64_t(e.value(QStringLiteral("fd")).toInteger(-1));
				if (p && p->frame())
					p->frame()->attach(fd, e.value(QStringLiteral("w")).toInt(), e.value(QStringLiteral("h")).toInt());
				else
					FrameView::closeHandle(fd);
			}
			else if (t == QLatin1String("frame") && p && p->frame())
				p->frame()->damage(e.value(QStringLiteral("x")).toInt(), e.value(QStringLiteral("y")).toInt(), e.value(QStringLiteral("w")).toInt(),
				                   e.value(QStringLiteral("h")).toInt());
			else if (t == QLatin1String("terminal"))
				m_dirtyTerminals.insert(s);
			else if (t == QLatin1String("question"))
			{
				// Rückfrage als Leiste im Tab der Sitzung; ohne Tab: ablehnen.
				const auto q = std::uint32_t(e.value(QStringLiteral("q")).toInt());
				if (p)
				{
					for (int i = 0; i < m_tabs->count(); ++i)
						if (m_tabs->widget(i)->isAncestorOf(p))
							m_tabs->setCurrentIndex(i);
					p->security()->showQuestion(int(q), e, [this, s, q](bool accept) { m_backend->answer(s, q, accept); });
				}
				else
					m_backend->answer(s, q, false);
			}
			else if (t == QLatin1String("question_closed"))
			{
				if (p)
					p->security()->closeQuestion(e.value(QStringLiteral("q")).toInt(), e.value(QStringLiteral("reason")).toString());
				if (e.value(QStringLiteral("reason")).toString() == QLatin1String("timeout"))
					statusBar()->showMessage(T(m_backend, "security.timed_out"), 8000);
			}
			else if (t == QLatin1String("notice"))
			{
				if (p)
					p->setNotice(e.value(QStringLiteral("text")).toString());
				updateNotice();
				statusBar()->showMessage(e.value(QStringLiteral("text")).toString(), 8000);
			}
			else if (t == QLatin1String("clipboard"))
				QGuiApplication::clipboard()->setText(e.value(QStringLiteral("text")).toString());
			else if (t == QLatin1String("clipboard_read_request"))
			{
				// Standard ist Ablehnen; Erlauben nur ausdrücklich.
				if (p)
					p->askBar()->ask(T(m_backend, "clipboard.read_request"), { T(m_backend, "clipboard.read_allow") }, [this, s](int b) {
						m_backend->clipboardReadAnswer(s, b == 0, b == 0 ? QGuiApplication::clipboard()->text() : QString());
					});
				else
					m_backend->clipboardReadAnswer(s, false, QString());
			}
			else if (t == QLatin1String("macro_done") && p)
				p->setMacroResult(e.value(QStringLiteral("message")).toString(), e.value(QStringLiteral("ok")).toBool());
			else if (t == QLatin1String("title") && p)
				p->setToolTip(e.value(QStringLiteral("title")).toString());
			else if (t == QLatin1String("web_status") && p)
			{
				p->setWebStatus(e);
				applyPageTitle(p);
				// Lade- und Fehlerzustand in der Fußzeile.
				const QString error = e.value(QStringLiteral("error")).toString();
				if (!error.isEmpty())
					statusBar()->showMessage(T(m_backend, "web.status.error").arg(p->title(), error), 15000);
				else if (e.value(QStringLiteral("loading")).toBool())
					statusBar()->showMessage(T(m_backend, "web.status.loading").arg(p->title()).arg(e.value(QStringLiteral("progress")).toInt()), 5000);
				else
					statusBar()->showMessage(T(m_backend, "web.status.loaded").arg(p->title()), 3000);
			}
			else if (t == QLatin1String("fields_detected"))
			{
				for (auto* ed : findChildren<PropertiesTab*>())
					if (ed->nodeId() == e.value(QStringLiteral("target")).toString())
						ed->setDetectedFields(e);
			}
			else if (t == QLatin1String("web_menu") && p)
				p->showWebMenu(e);
			else if (t == QLatin1String("web_focus") && p && p->frame())
				p->frame()->webFocus(e.value(QStringLiteral("on")).toBool(), e.value(QStringLiteral("forward")).toBool());
			else if (t == QLatin1String("web_key") && p)
				FrameView::triggerShortcut(this, e.value(QStringLiteral("vk")).toInt(), e.value(QStringLiteral("ctrl")).toBool(),
				                           e.value(QStringLiteral("alt")).toBool(), e.value(QStringLiteral("shift")).toBool());
			else if (t == QLatin1String("web_download") && p)
			{
				// Download einer Web-Sitzung (Windows): Fortschritt und Ergebnis in der Fußzeile, Fehler zusätzlich als Hinweis.
				const QString state = e.value(QStringLiteral("state")).toString();
				const QString name = e.value(QStringLiteral("name")).toString();
				const QString dir = QDir::toNativeSeparators(e.value(QStringLiteral("dir")).toString());
				const QString error = e.value(QStringLiteral("error")).toString();
				if (state == QLatin1String("started"))
					statusBar()->showMessage(T(m_backend, "web.download.started").arg(p->title(), name), 5000);
				else if (state == QLatin1String("progress"))
				{
					const qlonglong total = e.value(QStringLiteral("total")).toVariant().toLongLong();
					const qlonglong got = e.value(QStringLiteral("received")).toVariant().toLongLong();
					statusBar()->showMessage(T(m_backend, "web.download.progress").arg(p->title(), name).arg(total > 0 ? int(got * 100 / total) : 0), 3000);
				}
				else if (state == QLatin1String("done"))
				{
					const bool opened = e.value(QStringLiteral("opened")).toBool();
					const QString text = !error.isEmpty() ? T(m_backend, "web.download.open_failed").arg(p->title(), name, dir, error)
					                     : opened      ? T(m_backend, "web.download.opened").arg(p->title(), name, dir)
					                                   : T(m_backend, "web.download.done").arg(p->title(), name, dir);
					statusBar()->showMessage(text, 15000);
					// Wohin die Datei gekommen ist, soll stehen bleiben (Hinweiszeile), nicht nur kurz erscheinen.
					p->setNotice(text);
					updateNotice();
				}
				else if (state == QLatin1String("failed"))
				{
					const QString text = T(m_backend, "web.download.failed").arg(p->title(), name, error);
					statusBar()->showMessage(text, 15000);
					p->setNotice(text);
					updateNotice();
				}
			}
			else if (t == QLatin1String("tree_changed"))
				reloadTree();
			else if (t == QLatin1String("unlocked"))
			{
				m_overlay->hide();
				for (auto& w : m_windows)
					if (w)
						w->show();
				reloadViews();
				reloadTree();
				for (auto* ed : findChildren<PropertiesTab*>())
					ed->reload();
				statusBar()->showMessage(T(m_backend, "status.unlocked"), 5000);
				if (const QString note = m_backend->keyringNote(); !note.isEmpty())
					statusBar()->showMessage(note, 12000);
			}
			else if (t == QLatin1String("unlock_failed"))
			{
				if (!m_overlay->isVisible())
					showUnlock(false);
				m_overlay->setBusy(false, e.value(QStringLiteral("reason")).toString());
			}
			else if (t == QLatin1String("connect_all_done"))
			{
				rebuildTabs();
				statusBar()->showMessage(T(m_backend, "status.connect_all.done"), 5000);
			}
			else if (t == QLatin1String("session_opened"))
			{
				m_jobTargets.remove(qulonglong(e.value(QStringLiteral("job")).toInteger()));
				handleConnectResult(QString(), QString::fromUtf8(QJsonDocument(QJsonObject{ { QStringLiteral("session"), e.value(QStringLiteral("s")) } }).toJson()));
			}
			else if (t == QLatin1String("job_failed"))
			{
				const QString target = m_jobTargets.take(qulonglong(e.value(QStringLiteral("job")).toInteger()));
				QJsonObject r;
				if (e.value(QStringLiteral("hostkey")).isObject())
					r.insert(QStringLiteral("hostkey"), e.value(QStringLiteral("hostkey")));
				else
					r.insert(QStringLiteral("error"), e.value(QStringLiteral("error")));
				if (target.isEmpty())
					statusBar()->showMessage(e.value(QStringLiteral("error")).toString(), 8000);
				else
					handleConnectResult(target, QString::fromUtf8(QJsonDocument(r).toJson()));
			}
			else if (t == QLatin1String("job_result"))
			{
				const QString kind = e.value(QStringLiteral("kind")).toString();
				const QJsonObject r = parseObject(e.value(QStringLiteral("json")).toString());
				if (kind.startsWith(QLatin1String("update_")))
				{
					if (auto* u = findChild<UpdateTab*>())
						u->handleResult(kind, r);
					else if (kind == QLatin1String("update_check") && r.value(QStringLiteral("state")).toString() == QLatin1String("newer"))
						// Prüfung beim Start: nur ein Hinweis in der Fußzeile.
						statusBar()->showMessage(T(m_backend, "update.available").arg(r.value(QStringLiteral("found")).toObject().value(QStringLiteral("version")).toString()), 15000);
				}
				else if (kind == QLatin1String("guests"))
					applyGuests(e.value(QStringLiteral("target")).toString(), r);
				else if (kind == QLatin1String("cert_check"))
				{
					if (auto* c = findChild<CertCheckTab*>())
						c->handleResult(r);
				}
				else if (r.contains(QStringLiteral("error")))
					statusBar()->showMessage(T(m_backend, "edit.error").arg(r.value(QStringLiteral("error")).toString()), 8000);
				else if (kind == QLatin1String("guest_action"))
					statusBar()->showMessage(T(m_backend, "guest.action.sent"), 4000);
				else
					handleConnectResult(e.value(QStringLiteral("target")).toString(), e.value(QStringLiteral("json")).toString());
			}
			else if (t == QLatin1String("jobs_changed"))
				updateJobs();
			else if (t == QLatin1String("locked"))
			{
				lockNow(true);
				statusBar()->showMessage(T(m_backend, "status.locked.idle"));
			}
			else if (t == QLatin1String("resumed"))
				statusBar()->showMessage(T(m_backend, "status.resumed"), 8000);
		}
	}
}

void MainWindow::updateJobs()
{
	const QJsonArray jobs = QJsonDocument::fromJson(edit({ { QStringLiteral("op"), QStringLiteral("jobs") } }).toUtf8()).array();
	m_jobsBox->setVisible(!jobs.isEmpty());
	if (jobs.isEmpty())
		return;
	const QJsonObject first = jobs.first().toObject();
	QString text = T(m_backend, "jobs.running").arg(first.value(QStringLiteral("label")).toString()).arg(first.value(QStringLiteral("secs")).toInt());
	if (jobs.size() > 1)
		text += QStringLiteral(" · ") + T(m_backend, "jobs.more").arg(jobs.size());
	m_jobsLabel->setText(text);
}

// Lange Tab-Titel in der Mitte kürzen (Breite begrenzt das Stilblatt); der volle Titel erscheint als Hinweis.
void MainWindow::configureTabBar(QTabWidget* tabs)
{
	QTabBar* bar = tabs->tabBar();
	bar->setElideMode(Qt::ElideMiddle);
	bar->setUsesScrollButtons(true);
	bar->installEventFilter(this);
}

bool MainWindow::eventFilter(QObject* o, QEvent* e)
{
	if (m_compact && m_tabs && o == m_tabs->tabBar() && e->type() == QEvent::Resize)
		QTimer::singleShot(0, this, [this] { syncStripHeight(); });
	// Platzsparmodus: die freie Fläche der Tab-Leiste ist die Titelzeile (Ziehen, Doppelklick maximiert).
	// Ohne Tab hat die Leiste keine Fläche; dann gilt die Streifenhöhe oben im Tab-Widget selbst.
	if (m_compact && m_tabs && (o == m_tabs->tabBar() || o == m_tabs) && (e->type() == QEvent::MouseButtonPress || e->type() == QEvent::MouseButtonDblClick))
	{
		auto* me = static_cast<QMouseEvent*>(e);
		const QPoint pt = me->position().toPoint();
		const bool inStrip = o == m_tabs ? pt.y() < qMax(26, m_titleBar->controls()->height()) : m_tabs->tabBar()->tabAt(pt) < 0;
		if (me->button() == Qt::LeftButton && inStrip)
		{
			if (e->type() == QEvent::MouseButtonDblClick)
			{
				toggleMaximized(this);
				m_titleBar->updateState();
			}
			else if (windowHandle())
				windowHandle()->startSystemMove();
			return true;
		}
	}
	if (e->type() == QEvent::ToolTip)
		if (auto* bar = qobject_cast<QTabBar*>(o))
		{
			const int i = bar->tabAt(static_cast<QHelpEvent*>(e)->pos());
			if (i >= 0)
			{
				QString full = bar->tabToolTip(i);
				if (full.isEmpty())
					full = bar->tabText(i);
				QToolTip::showText(static_cast<QHelpEvent*>(e)->globalPos(), full.remove(QLatin1Char('&')), bar);
				return true;
			}
		}
	if (m_tree && o == m_tree->viewport() && e->type() == QEvent::Resize)
		updateEmptyHint();
	// Zeiger am Fensterrand (Größenänderung): Er wird nur dort umgestellt und sonst sofort zurückgenommen.
	// Ein am Fenster gesetzter Zeiger würde sonst von allen Kindern geerbt (Terminal, RDP-Bild).
	if (e->type() == QEvent::MouseMove && o->isWidgetType() && static_cast<QWidget*>(o)->window() == this)
		updateEdgeCursor(static_cast<QMouseEvent*>(e)->globalPosition().toPoint());
	else if ((e->type() == QEvent::Leave || e->type() == QEvent::WindowDeactivate) && o == this && m_edgeOverride)
		updateEdgeCursor(QPoint(-100000, -100000));
	switch (e->type())
	{
	case QEvent::KeyPress:
	case QEvent::MouseButtonPress:
	case QEvent::Wheel:
	case QEvent::MouseMove:
		// Höchstens einmal je Sekunde an den Leerlauf-Zeitgeber melden.
		if (m_activityClock.elapsed() - m_lastActivity > 1000)
		{
			m_lastActivity = m_activityClock.elapsed();
			m_backend->activity();
		}
		break;
	default:
		break;
	}
	return QMainWindow::eventFilter(o, e);
}

void MainWindow::onPrepareForSleep(bool sleeping)
{
	if (!sleeping)
		systemResumed();
}

void MainWindow::systemResumed()
{
	handleEvents(m_backend->systemSleep(false));
}

void MainWindow::lockNow(bool alreadyLocked)
{
	if (!alreadyLocked)
		m_backend->lock();
	// Abgerissene Fenster zeigen Sitzungsbilder: ebenfalls verbergen.
	for (auto& w : m_windows)
		if (w)
			w->hide();
	for (auto* ed : findChildren<PropertiesTab*>())
		ed->wipeSecrets();
	for (auto* st : findChildren<SnmpTab*>())
		st->wipeSecrets();
	for (auto* tt : findChildren<ToolTab*>())
		tt->wipeSecrets();
	reloadTree();
	statusBar()->showMessage(T(m_backend, "status.locked"));
	showUnlock(true);
}

void MainWindow::idleSettings()
{
	const QJsonObject cur = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("settings") } }));
	QList<QPair<QString, QString>> items{ { T(m_backend, "idle.off"), QStringLiteral("0") } };
	for (int m : { 5, 10, 15, 30, 60 })
		items.append({ T(m_backend, "idle.minutes").arg(m), QString::number(m) });
	m_bar->choose(T(m_backend, "bar.idle"), items, nullptr, T(m_backend, "bar.idle.disconnect"), [this](const QString& v, bool disconnect) {
		edit({ { QStringLiteral("op"), QStringLiteral("set_settings") },
		       { QStringLiteral("lock_after_idle_mins"), v.toInt() },
		       { QStringLiteral("disconnect_on_lock"), disconnect } });
		statusBar()->showMessage(T(m_backend, "status.idle").arg(v == QLatin1String("0") ? T(m_backend, "idle.off") : T(m_backend, "idle.minutes").arg(v)), 5000);
	});
	const int i = m_bar->combo()->findData(QString::number(cur.value(QStringLiteral("lock_after_idle_mins")).toInt()));
	if (i >= 0)
		m_bar->combo()->setCurrentIndex(i);
	if (auto* c = m_bar->findChild<QCheckBox*>())
		c->setChecked(cur.value(QStringLiteral("disconnect_on_lock")).toBool());
}

void MainWindow::showCredentialChoiceForScreenshot()
{
	QJsonArray choices;
	choices.append(QJsonObject{ { QStringLiteral("name"), QStringLiteral("admin") }, { QStringLiteral("title"), QStringLiteral("Verwaltung Standort Nord") } });
	choices.append(QJsonObject{ { QStringLiteral("name"), QStringLiteral("user") }, { QStringLiteral("title"), QStringLiteral("Benutzerkonto") } });
	PropertiesTab* ed = openFirstTargetProperties();
	const QString first = ed ? ed->nodeId() : QString();
	if (ed)
	{
		m_tabs->removeTab(m_tabs->indexOf(ed));
		ed->deleteLater();
	}
	ConnectPromptTab* tab = promptTab(first);
	tab->setObjectName(QStringLiteral("credentialPrompt"));
	chooseCredential(tab, choices);
}

QWidget* MainWindow::showHostKeyQuestionForScreenshot(bool changed)
{
	if (m_panes.isEmpty())
		return nullptr;
	SessionPane* p = m_panes.begin().value();
	for (int i = 0; i < m_tabs->count(); ++i)
		if (m_tabs->widget(i)->isAncestorOf(p))
			m_tabs->setCurrentIndex(i);
	QJsonObject info{ { QStringLiteral("kind"), QStringLiteral("hostkey") },
		              { QStringLiteral("host"), QStringLiteral("srv01.example.test") },
		              { QStringLiteral("port"), 22 },
		              { QStringLiteral("key_type"), QStringLiteral("ssh-ed25519") },
		              { QStringLiteral("fingerprint"), QStringLiteral("SHA256:q3e5V0bS3vLz2k8l1cYxN4hG6tJ9mWfQ7rUaPdE0oKs") } };
	QString text = QStringLiteral("Der Host-Schlüssel von srv01.example.test:22 ist unbekannt. Prüfen Sie den Fingerabdruck, bevor Sie ihn annehmen.");
	if (changed)
	{
		info.insert(QStringLiteral("previous"), QStringLiteral("SHA256:Zm9vYmFyYmF6cXV4ZXhhbXBsZWZpbmdlcnByaW50MDE"));
		text = QStringLiteral("WARNUNG: Der Host-Schlüssel von srv01.example.test:22 hat sich geändert. Das kann auf einen Angriff hinweisen.");
	}
	p->security()->showQuestion(changed ? 2 : 1, QJsonObject{ { QStringLiteral("text"), text }, { QStringLiteral("info"), info }, { QStringLiteral("timeout_secs"), 120 } },
	                            [](bool) {});
	return p;
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

// Rand, an dem die Größe verändert wird (das Fenster hat keinen Systemrahmen).
static Qt::Edges edgesAt(const QWidget* w, const QPoint& p)
{
	Qt::Edges e;
	if (p.x() < kResizeBorder)
		e |= Qt::LeftEdge;
	if (p.x() >= w->width() - kResizeBorder)
		e |= Qt::RightEdge;
	if (p.y() < kResizeBorder)
		e |= Qt::TopEdge;
	if (p.y() >= w->height() - kResizeBorder)
		e |= Qt::BottomEdge;
	return e;
}

static Qt::CursorShape cursorFor(Qt::Edges e)
{
	if (e == (Qt::LeftEdge | Qt::TopEdge) || e == (Qt::RightEdge | Qt::BottomEdge))
		return Qt::SizeFDiagCursor;
	if (e == (Qt::RightEdge | Qt::TopEdge) || e == (Qt::LeftEdge | Qt::BottomEdge))
		return Qt::SizeBDiagCursor;
	if (e & (Qt::LeftEdge | Qt::RightEdge))
		return Qt::SizeHorCursor;
	if (e & (Qt::TopEdge | Qt::BottomEdge))
		return Qt::SizeVerCursor;
	return Qt::ArrowCursor;
}

void MainWindow::updateEdgeCursor(const QPoint& global)
{
	const Qt::Edges edges = isMaximized() || isFullScreen() ? Qt::Edges() : edgesAt(this, mapFromGlobal(global));
	if (edges)
	{
		if (m_edgeOverride)
			QGuiApplication::changeOverrideCursor(QCursor(cursorFor(edges)));
		else
			QGuiApplication::setOverrideCursor(QCursor(cursorFor(edges)));
		m_edgeOverride = true;
	}
	else if (m_edgeOverride)
	{
		QGuiApplication::restoreOverrideCursor();
		m_edgeOverride = false;
	}
}

void MainWindow::mouseMoveEvent(QMouseEvent* e)
{
	QMainWindow::mouseMoveEvent(e);
}

void MainWindow::mousePressEvent(QMouseEvent* e)
{
	if (e->button() == Qt::LeftButton && !isMaximized() && !isFullScreen())
	{
		const Qt::Edges edges = edgesAt(this, e->position().toPoint());
		if (edges && windowHandle())
		{
			windowHandle()->startSystemResize(edges);
			e->accept();
			return;
		}
	}
	QMainWindow::mousePressEvent(e);
}

void MainWindow::changeEvent(QEvent* e)
{
	QMainWindow::changeEvent(e);
	if (e->type() == QEvent::WindowStateChange)
	{
		// Maximiert: kein Rand zum Ziehen.
		const int m = isMaximized() || isFullScreen() ? 0 : kResizeBorder;
		setContentsMargins(m, m, m, m);
		if (m_titleBar)
			m_titleBar->updateState();
	}
}

/// Beim Start: mit dem im Schlüsselbund gemerkten Hauptkennwort entsperren (nur wenn eingeschaltet).
void MainWindow::tryKeyringUnlock()
{
	if (m_backend->unlockFromKeyring())
	{
		m_overlay->setBusy(true, QString());
		statusBar()->showMessage(T(m_backend, "status.keyring.unlocking"));
	}
}

void MainWindow::forgetKeyring()
{
	statusBar()->showMessage(m_backend->keyringForget(), 10000);
}

void MainWindow::restoreWindowState()
{
	const QJsonObject s = parseObject(m_backend->windowStateJson());
	if (!s.value(QStringLiteral("saved")).toBool())
		return;
	const int w = s.value(QStringLiteral("width")).toInt();
	const int h = s.value(QStringLiteral("height")).toInt();
	if (w < 400 || h < 300)
		return;
	// Nie größer als der sichtbare Bereich des Bildschirms (Dock, Monitorwechsel).
	QSize size(w, h);
	if (QScreen* sc = QGuiApplication::primaryScreen())
	{
		const QRect a = sc->availableGeometry();
		size = size.boundedTo(a.size());
	}
	resize(size);
	// Unter Wayland darf eine Anwendung ihre Lage nicht selbst setzen; dort gilt nur die Größe.
	// Sonst nur, wenn die Lage auf einem vorhandenen Bildschirm liegt.
	const QPoint pos(s.value(QStringLiteral("x")).toInt(), s.value(QStringLiteral("y")).toInt());
	if (QGuiApplication::platformName() != QLatin1String("wayland") && QGuiApplication::screenAt(pos + QPoint(40, 40)))
		move(pos);
	const int tree = s.value(QStringLiteral("tree_width")).toInt();
	if (m_split && tree >= 100)
		m_split->setSizes({ tree, qMax(200, size.width() - tree) });
	if (s.value(QStringLiteral("maximized")).toBool())
		setWindowState(windowState() | Qt::WindowMaximized);
	setTreeVisible(!s.value(QStringLiteral("tree_hidden")).toBool());
	setCompact(s.value(QStringLiteral("compact")).toBool());
	fitToScreen();
}

void MainWindow::saveWindowState()
{
	// Bei maximiertem Fenster die normale Größe merken, damit sie nach dem Verkleinern stimmt.
	const QRect g = (isMaximized() || isFullScreen()) ? normalGeometry() : geometry();
	QJsonObject o;
	o.insert(QStringLiteral("x"), g.x());
	o.insert(QStringLiteral("y"), g.y());
	o.insert(QStringLiteral("width"), g.width());
	o.insert(QStringLiteral("height"), g.height());
	o.insert(QStringLiteral("maximized"), isMaximized());
	// Bei eingeklapptem Baum bleibt die zuletzt gemerkte Breite erhalten.
	if (m_split && !m_split->sizes().isEmpty() && m_split->sizes().first() >= 100)
		m_treeWidth = m_split->sizes().first();
	o.insert(QStringLiteral("tree_width"), m_treeWidth);
	o.insert(QStringLiteral("compact"), m_compact);
	o.insert(QStringLiteral("tree_hidden"), m_treeHidden);
	m_backend->saveWindowState(QString::fromUtf8(QJsonDocument(o).toJson(QJsonDocument::Compact)));
}

void MainWindow::screenLayoutChanged()
{
	// Mehrere Signale beim Umstecken des Docks: erst nach kurzer Ruhe prüfen.
	m_fitTimer.start();
}

void MainWindow::fitToScreen()
{
	if (isMaximized() || isFullScreen())
		return;
	QScreen* sc = screen();
	const QRect frame = frameGeometry();
	// Liegt das Fenster auf keinem Bildschirm mehr (Monitor abgesteckt), auf den Hauptbildschirm holen.
	bool onScreen = false;
	for (QScreen* x : QGuiApplication::screens())
		if (x->availableGeometry().intersects(QRect(frame.center() - QPoint(20, 20), QSize(40, 40))))
		{
			sc = x;
			onScreen = true;
			break;
		}
	if (!onScreen || !sc)
		sc = QGuiApplication::primaryScreen();
	if (!sc)
		return;
	const QRect a = sc->availableGeometry();
	const QSize extra = frame.size() - size();
	const QSize maxSize = a.size() - extra;
	QSize target = size().boundedTo(maxSize).expandedTo(QSize(400, 300));
	if (target != size())
		resize(target);
	if (QGuiApplication::platformName() == QLatin1String("wayland"))
		return;
	QRect f = frameGeometry();
	if (!onScreen)
		f.moveTopLeft(a.topLeft());
	// Mindestens die Titelzeile muss im sichtbaren Bereich liegen.
	if (f.right() > a.right())
		f.moveRight(a.right());
	if (f.bottom() > a.bottom())
		f.moveBottom(a.bottom());
	if (f.left() < a.left())
		f.moveLeft(a.left());
	if (f.top() < a.top())
		f.moveTop(a.top());
	if (f.topLeft() != frameGeometry().topLeft())
		move(f.topLeft() + (pos() - frameGeometry().topLeft()));
}

void MainWindow::resetWindow()
{
	if (isMaximized() || isFullScreen())
		showNormal();
	QScreen* sc = screen() ? screen() : QGuiApplication::primaryScreen();
	const QRect a = sc ? sc->availableGeometry() : QRect(0, 0, 1280, 800);
	m_treeWidth = 280;
	setTreeVisible(true);
	setCompact(false);
	resize(QSize(1280, 800).boundedTo(a.size() - (frameGeometry().size() - size())));
	if (m_split)
		m_split->setSizes({ 280, qMax(200, width() - 280) });
	if (QGuiApplication::platformName() != QLatin1String("wayland"))
		move(a.center() - QPoint(width() / 2, height() / 2));
	statusBar()->showMessage(T(m_backend, "status.window.reset"), 6000);
}

void MainWindow::setTreeVisible(bool on)
{
	if (!on && m_split && !m_split->sizes().isEmpty() && m_split->sizes().first() >= 100 && !m_treeHidden && m_leftInner->isVisible())
		m_treeWidth = m_split->sizes().first();
	m_treeHidden = !on;
	if (m_treeButton)
	{
		m_treeButton->setText(on ? QStringLiteral("\u00ab") : QStringLiteral("\u00bb"));
		const QString tip = T(m_backend, on ? "status.tree.hide" : "status.tree.show");
		m_treeButton->setToolTip(tip);
		m_treeButton->setAccessibleName(tip);
	}
	placeBrand();
}

void MainWindow::setCompact(bool on)
{
	if (m_compactSwitch && m_compactSwitch->isChecked() != on)
	{
		QSignalBlocker b(m_compactSwitch);
		m_compactSwitch->setChecked(on);
	}
	if (m_compactAction && m_compactAction->isChecked() != on)
		m_compactAction->setChecked(on);
	if (m_compact == on && m_titleBar->controls()->parentWidget() != m_titleBar)
		return;
	m_compact = on;
	// Niedrigere Tabs im Platzsparmodus (Stylesheet: QTabBar[compact="true"]).
	m_tabs->tabBar()->setProperty("compact", on);
	m_tabs->tabBar()->style()->unpolish(m_tabs->tabBar());
	m_tabs->tabBar()->style()->polish(m_tabs->tabBar());
	QWidget* top = m_menuBar->parentWidget();
	if (on)
	{
		top->hide();
		m_titleBar->controls()->setBurgerMenu(m_burgerMenu);
		m_titleBar->controls()->setBurgerVisible(true);
		m_tabs->setCornerWidget(m_titleBar->controls(), Qt::TopRightCorner);
		m_titleBar->controls()->show();
	}
	else
	{
		m_tabs->setCornerWidget(nullptr, Qt::TopRightCorner);
		m_brandSlotLay->removeWidget(m_titleBar->brand());
		m_titleBar->restoreParts();
		m_brandSlot->hide();
		top->show();
	}
	placeBrand();
	m_titleBar->updateState();
}

/// Baum und Logo-Streifen anordnen. Im Platzsparmodus steht Logo, Name und Version bei angepinntem Baum
/// oben in der linken Spalte; bei eingeklapptem Baum links neben den Tabs, und die Sitzungsfläche reicht
/// darunter bis zum Fensterrand.
void MainWindow::placeBrand()
{
	if (!m_leftPane || !m_split)
		return;
	const bool on = !m_treeHidden;
	if (!m_compact)
	{
		m_leftPane->setMinimumWidth(0);
		m_leftPane->setMaximumWidth(QWIDGETSIZE_MAX);
		m_leftInner->setVisible(true);
		m_leftPane->setVisible(on);
		if (on)
			m_split->setSizes({ m_treeWidth, qMax(200, m_split->width() - m_treeWidth) });
		return;
	}
	BrandStrip* brand = m_titleBar->brand();
	brand->setStyleSheet(QStringLiteral("QWidget#brandStrip { background: palette(button); border-bottom: 1px solid palette(mid); }"
	                                    "QWidget#brandStrip QLabel { background: transparent; }"));
	brand->setAttribute(Qt::WA_StyledBackground, true);
	m_leftPane->setVisible(on);
	m_leftInner->setVisible(true);
	if (on)
	{
		m_tabs->setCornerWidget(nullptr, Qt::TopLeftCorner);
		if (brand->parentWidget() != m_brandSlot)
		{
			brand->setParent(m_brandSlot);
			m_brandSlotLay->addWidget(brand);
		}
		m_brandSlot->show();
		m_split->setSizes({ m_treeWidth, qMax(200, m_split->width() - m_treeWidth) });
	}
	else
	{
		m_brandSlotLay->removeWidget(brand);
		m_brandSlot->hide();
		m_tabs->setCornerWidget(brand, Qt::TopLeftCorner);
	}
	brand->show();
	syncStripHeight();
	QTimer::singleShot(0, this, [this] { syncStripHeight(); });
}

/// Logo-Streifen und Fensterknöpfe so hoch wie die Tab-Leiste, damit alles auf einer Linie liegt.
void MainWindow::syncStripHeight()
{
	if (!m_compact)
		return;
	// Die Ecken-Widgets enden an der Oberkante der Seite; so hoch wie dieser Abstand liegen sie bündig zu den Tabs.
	const int bh = m_tabs->tabBar()->height();
	const int py = m_tabs->currentWidget() ? m_tabs->currentWidget()->mapTo(m_tabs, QPoint(0, 0)).y() : 0;
	const int h = qMax(26, py > 16 ? py : (bh > 16 ? bh : m_tabs->tabBar()->sizeHint().height()));
	bool changed = false;
	for (QWidget* w : { static_cast<QWidget*>(m_titleBar->brand()), static_cast<QWidget*>(m_titleBar->controls()) })
		if (w->height() != h || w->minimumHeight() != h)
		{
			w->setFixedHeight(h);
			changed = true;
		}
	// Die Ecken-Widgets werden erst bei einem neuen Layout der Tab-Leiste an die Höhe angepasst.
	if (changed)
	{
		QResizeEvent ev(m_tabs->size(), m_tabs->size());
		QApplication::sendEvent(m_tabs, &ev);
	}
}

/// Hinweis der vorderen Sitzung in der Mitte der Fußzeile (statt der orangen Zeile in der Sitzung).
void MainWindow::updateNotice()
{
	if (!m_noticeLabel)
		return;
	// Zustand (nicht „Verbunden“) und Hinweis der Sitzungen im vorderen Tab; schwere Fehler rot und fett.
	QStringList parts;
	bool severe = false;
	if (QWidget* cur = m_tabs->currentWidget())
	{
		QList<SessionPane*> here;
		for (SessionPane* p : std::as_const(m_panes))
			if (p && (p == cur || cur->isAncestorOf(p)))
				here.append(p);
		std::sort(here.begin(), here.end(), [](SessionPane* a, SessionPane* b) { return a->session() < b->session(); });
		for (SessionPane* p : here)
		{
			QString line = p->statusLine();
			if (line.isEmpty() && p->notice().isEmpty())
				continue;
			if (!p->notice().isEmpty())
				line += (line.isEmpty() ? QString() : QStringLiteral(" · ")) + p->notice();
			if (here.size() > 1)
				line = p->title() + QStringLiteral(": ") + line;
			severe = severe || p->severe();
			parts.append(line);
		}
	}
	const QString text = parts.join(QStringLiteral("   |   "));
	m_noticeLabel->setProperty("severe", severe);
	m_noticeLabel->style()->unpolish(m_noticeLabel);
	m_noticeLabel->style()->polish(m_noticeLabel);
	m_noticeLabel->setText(text);
	m_noticeLabel->setToolTip(text);
}

void MainWindow::closeEvent(QCloseEvent* e)
{
	// Ungespeicherte Änderungen (zum Beispiel ein Import) gehen sonst beim Beenden verloren.
	if (m_backend->hasUnsavedChanges())
	{
		QMessageBox box(QMessageBox::Warning, T(m_backend, "close.unsaved.title"), T(m_backend, "close.unsaved.text"),
		                QMessageBox::NoButton, this);
		auto* save = box.addButton(T(m_backend, "close.unsaved.save"), QMessageBox::AcceptRole);
		auto* discard = box.addButton(T(m_backend, "close.unsaved.discard"), QMessageBox::DestructiveRole);
		box.addButton(T(m_backend, "close.unsaved.cancel"), QMessageBox::RejectRole);
		box.setDefaultButton(save);
		box.exec();
		if (box.clickedButton() == save)
		{
			const QString r = m_backend->save();
			if (r.startsWith(QStringLiteral("Speichern fehlgeschlagen")))
			{
				statusBar()->showMessage(r, 10000);
				e->ignore();
				return;
			}
		}
		else if (box.clickedButton() != discard)
		{
			e->ignore();
			return;
		}
	}
	saveWindowState();
	for (auto& w : m_windows)
		if (w)
			w->close();
	QMainWindow::closeEvent(e);
}

// ------------------------------------------------------------- Bearbeiten

QString MainWindow::edit(const QJsonObject& cmd)
{
	return m_backend->edit(QString::fromUtf8(QJsonDocument(cmd).toJson(QJsonDocument::Compact)));
}

void MainWindow::showEditResult(const QJsonObject& r, const QString& okMessage)
{
	if (r.contains(QStringLiteral("error")))
		statusBar()->showMessage(T(m_backend, "edit.error").arg(r.value(QStringLiteral("error")).toString()), 8000);
	else if (!okMessage.isEmpty())
		statusBar()->showMessage(okMessage, 5000);
	reloadTree();
	for (auto* ed : findChildren<PropertiesTab*>())
		if (!ed->isDirty())
			ed->reload();
}

QString MainWindow::currentId() const
{
	const QModelIndex i = m_tree->currentIndex();
	if (!i.isValid() || i.data(RoleGuest).isValid() || i.data(RolePlaceholder).toBool() || i.data(RoleMacro).toInt())
		return QString();
	return i.data(RoleId).toString();
}

QString MainWindow::currentParentId() const
{
	const QModelIndex i = m_tree->currentIndex();
	if (!i.isValid() || i.data(RoleMacro).toInt())
		return QString();
	if (i.data(RoleFolder).toBool() && !i.data(RolePlaceholder).toBool())
		return i.data(RoleId).toString();
	return i.parent().isValid() ? i.parent().data(RoleId).toString() : QString();
}

QStringList MainWindow::selectedIds() const
{
	QStringList ids;
	for (const QModelIndex& i : m_tree->selectionModel()->selectedIndexes())
		if (!i.data(RoleGuest).isValid() && !i.data(RolePlaceholder).toBool() && !i.data(RoleMacro).toInt())
			ids << i.data(RoleId).toString();
	if (ids.isEmpty() && !currentId().isEmpty())
		ids << currentId();
	return ids;
}

void MainWindow::selectId(const QString& id)
{
	std::function<bool(const QModelIndex&)> walk = [&](const QModelIndex& parent) {
		for (int r = 0; r < m_model->rowCount(parent); ++r)
		{
			const QModelIndex i = m_model->index(r, 0, parent);
			if (i.data(RoleId).toString() == id && !i.data(RoleGuest).isValid())
			{
				m_tree->setCurrentIndex(i);
				m_tree->scrollTo(i);
				return true;
			}
			if (walk(i))
				return true;
		}
		return false;
	};
	walk(QModelIndex());
}

void MainWindow::buildAddMenu(QMenu* menu, const QString& parentId)
{
	const char* kinds[][2] = { { "folder", "add.folder" }, { "rdp", "add.rdp" },         { "ssh", "add.ssh" },     { "vnc", "add.vnc" },
		                       { "web", "add.web" },       { "proxmox", "add.proxmox" }, { "hyperv", "add.hyperv" }, { "psrp", "add.psrp" },
		                       { "task", "add.task" },     { "macro", "add.macro" } };
	auto run = [this, parentId](QJsonObject cmd) {
		if (!parentId.isEmpty())
			cmd.insert(QStringLiteral("parent"), parentId);
		const QJsonObject r = parseObject(edit(cmd));
		showEditResult(r);
		if (r.contains(QStringLiteral("id")))
		{
			const QString id = r.value(QStringLiteral("id")).toString();
			selectId(id);
			openProperties(id, QStringLiteral("general"));
		}
		else if ((r.contains(QStringLiteral("task")) || r.contains(QStringLiteral("macro"))))
			openProperties(parentId.isEmpty() ? QString() : parentId, QStringLiteral("tasks"));
	};
	for (auto& k : kinds)
	{
		const QString kind = QString::fromLatin1(k[0]);
		if (kind == QLatin1String("task"))
			menu->addSeparator();
		QAction* a = menu->addAction(m_backend->text(QString::fromLatin1(k[1])), this, [run, kind] {
			run({ { QStringLiteral("op"), QStringLiteral("add") }, { QStringLiteral("kind"), kind } });
		});
		a->setObjectName(QStringLiteral("add_") + kind);
		a->setIcon(protocolIcon(kind, QString(), kind == QLatin1String("folder")));
		if ((kind == QLatin1String("task") || kind == QLatin1String("macro")) && parentId.isEmpty())
			a->setEnabled(true);
	}
	menu->addSeparator();
	QAction* bulk = menu->addAction(T(m_backend, "add.bulk"), this, [this, parentId] { openBulkAdd(parentId); });
	bulk->setObjectName(QStringLiteral("add_bulk"));
	QMenu* tpl = menu->addMenu(T(m_backend, "add.template"));
	const QJsonArray list = QJsonDocument::fromJson(edit({ { QStringLiteral("op"), QStringLiteral("templates") } }).toUtf8()).array();
	if (list.isEmpty())
		tpl->addAction(T(m_backend, "add.template.none"))->setEnabled(false);
	for (const QJsonValue& v : list)
	{
		const QJsonObject o = v.toObject();
		const QString tid = o.value(QStringLiteral("id")).toString();
		tpl->addAction(protocolIcon(o.value(QStringLiteral("protocol")).toString(), QString(), false),
		               o.value(QStringLiteral("name")).toString() + (o.value(QStringLiteral("is_default")).toBool() ? QStringLiteral(" ★") : QString()), this,
		               [run, tid] { run({ { QStringLiteral("op"), QStringLiteral("add") }, { QStringLiteral("template"), tid } }); });
	}
}

QMenu* MainWindow::popupAddMenu()
{
	auto* menu = new QMenu(this);
	menu->setObjectName(QStringLiteral("addMenu"));
	menu->setAttribute(Qt::WA_DeleteOnClose);
	buildAddMenu(menu, currentParentId());
	const QRect r = m_tree->visualRect(m_tree->currentIndex());
	menu->popup(m_tree->viewport()->mapToGlobal(r.isValid() ? r.bottomLeft() : QPoint(0, 0)));
	return menu;
}

PropertiesTab* MainWindow::openProperties(const QString& id, const QString& category)
{
	QString target = id;
	if (target.isEmpty())
	{
		// Wurzel: ID über den Editor ermitteln.
		target = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("editor") }, { QStringLiteral("id"), QStringLiteral("00000000-0000-0000-0000-000000000000") } })).value(QStringLiteral("id")).toString();
		if (target.isEmpty())
			return nullptr;
	}
	for (int i = 0; i < m_tabs->count(); ++i)
		if (auto* ed = qobject_cast<PropertiesTab*>(m_tabs->widget(i)); ed && ed->nodeId() == target)
		{
			m_tabs->setCurrentIndex(i);
			if (!category.isEmpty())
				ed->selectCategory(category);
			return ed;
		}
	auto* ed = new PropertiesTab(m_backend, target, m_tabs);
	const int idx = m_tabs->addTab(ed, ed->title());
	m_tabs->setTabToolTip(idx, T(m_backend, "edit.cat.general"));
	connect(ed, &PropertiesTab::titleChanged, this, [this, ed] {
		const int i = m_tabs->indexOf(ed);
		if (i >= 0)
			m_tabs->setTabText(i, ed->title());
	});
	connect(ed, &PropertiesTab::indexChanged, this, [this, ed] {
		reloadTree();
		reloadViews();
		for (auto* other : findChildren<PropertiesTab*>())
			if (other != ed && !other->isDirty())
				other->reload();
	});
	connect(ed, &PropertiesTab::status, this, [this](const QString& t) { statusBar()->showMessage(t, 5000); });
	connect(ed, &PropertiesTab::saveClosed, this, [this, ed] {
		const int i = m_tabs->indexOf(ed);
		if (i >= 0)
		{
			m_tabs->removeTab(i);
			ed->deleteLater();
		}
	});
	m_tabs->setCurrentWidget(ed);
	if (!category.isEmpty())
		ed->selectCategory(category);
	return ed;
}

BulkAddTab* MainWindow::openBulkAdd(const QString& parentId)
{
	auto* t = new BulkAddTab(m_backend, parentId, m_tabs);
	addToolTab(t, T(m_backend, "bulk.title"));
	t->hosts()->setFocus();
	return t;
}

PropertiesTab* MainWindow::openFirstTargetProperties(const QString& protocol)
{
	std::function<QString(const QModelIndex&)> walk = [&](const QModelIndex& parent) -> QString {
		for (int r = 0; r < m_model->rowCount(parent); ++r)
		{
			const QModelIndex i = m_model->index(r, 0, parent);
			if (!i.data(RoleFolder).toBool() && i.data(RoleProtocol).toString() == protocol)
				return i.data(RoleId).toString();
			const QString f = walk(i);
			if (!f.isEmpty())
				return f;
		}
		return QString();
	};
	const QString id = walk(QModelIndex());
	selectId(id);
	return id.isEmpty() ? nullptr : openProperties(id);
}

void MainWindow::renameSelected()
{
	const QString id = currentId();
	if (id.isEmpty())
		return;
	const QString name = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("editor") }, { QStringLiteral("id"), id } })).value(QStringLiteral("name")).toString();
	m_bar->prompt(T(m_backend, "bar.rename").arg(m_tree->currentIndex().data().toString()), name, [this, id](const QString& n) {
		showEditResult(parseObject(edit({ { QStringLiteral("op"), QStringLiteral("rename") }, { QStringLiteral("id"), id }, { QStringLiteral("name"), n } })));
		selectId(id);
		m_tree->setFocus();
	});
}

void MainWindow::moveSelected()
{
	const QString id = currentId();
	if (id.isEmpty())
		return;
	const QJsonObject e = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("editor") }, { QStringLiteral("id"), id } }));
	QList<QPair<QString, QString>> items;
	for (const QJsonValue& f : e.value(QStringLiteral("folders")).toArray())
		items.append({ f.toObject().value(QStringLiteral("path")).toString(), f.toObject().value(QStringLiteral("id")).toString() });
	m_bar->choose(T(m_backend, "bar.move").arg(m_tree->currentIndex().data().toString()), items, [this, id](const QString& parent) {
		showEditResult(parseObject(edit({ { QStringLiteral("op"), QStringLiteral("move") }, { QStringLiteral("id"), id }, { QStringLiteral("parent"), parent } })));
		selectId(id);
		m_tree->setFocus();
	});
	const int cur = m_bar->combo()->findData(e.value(QStringLiteral("parent")).toString());
	if (cur >= 0)
		m_bar->combo()->setCurrentIndex(cur);
}

void MainWindow::deleteSelected()
{
	const QStringList ids = selectedIds();
	if (ids.isEmpty())
		return;
	QString text = ids.size() == 1 ? T(m_backend, "bar.delete").arg(m_tree->currentIndex().data().toString()) : T(m_backend, "bar.delete.many").arg(ids.size());
	// Offene Sitzungen der betroffenen Ziele (auch in Unterordnern) werden beendet; darauf hinweisen.
	const QJsonObject preview = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("delete_preview") }, { QStringLiteral("ids"), QJsonArray::fromStringList(ids) } }));
	if (const int active = preview.value(QStringLiteral("active")).toInt(); active > 0)
		text += QLatin1Char(' ') + T(m_backend, "bar.delete.sessions").arg(active);
	m_bar->ask(text, { T(m_backend, "bar.delete.button") }, [this, ids](int i) {
		if (i != 0)
			return;
		// Offene Eigenschaften-Tabs gelöschter Objekte schließen.
		for (auto* ed : findChildren<PropertiesTab*>())
			if (ids.contains(ed->nodeId()))
			{
				m_tabs->removeTab(m_tabs->indexOf(ed));
				ed->deleteLater();
			}
		const QJsonObject result = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("delete") }, { QStringLiteral("ids"), QJsonArray::fromStringList(ids) } }));
		// Beendete Sitzungen: Ansichten und Tabs entfernen.
		const QJsonArray closed = result.value(QStringLiteral("closed_sessions")).toArray();
		for (const QJsonValue& s : closed)
			if (auto* p = m_panes.take(std::uint64_t(s.toInteger())))
				p->deleteLater();
		showEditResult(result);
		if (!closed.isEmpty())
			rebuildTabs();
		m_tree->setFocus();
	});
}

void MainWindow::duplicateSelected()
{
	const QString id = currentId();
	if (id.isEmpty())
		return;
	const QJsonObject r = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("duplicate") }, { QStringLiteral("id"), id } }));
	showEditResult(r);
	selectId(r.value(QStringLiteral("id")).toString());
}

void MainWindow::toggleFavorite()
{
	const QString id = currentId();
	if (id.isEmpty())
		return;
	const bool fav = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("editor") }, { QStringLiteral("id"), id } })).value(QStringLiteral("favorite")).toBool();
	showEditResult(parseObject(edit({ { QStringLiteral("op"), QStringLiteral("favorite") }, { QStringLiteral("id"), id }, { QStringLiteral("on"), !fav } })),
	               T(m_backend, fav ? "ctx.favorite.off" : "ctx.favorite.on").remove(QLatin1Char('&')));
	selectId(id);
}

void MainWindow::reorderSelected(int delta)
{
	const QString id = currentId();
	if (id.isEmpty())
		return;
	showEditResult(parseObject(edit({ { QStringLiteral("op"), QStringLiteral("reorder") }, { QStringLiteral("id"), id }, { QStringLiteral("delta"), delta } })));
	selectId(id);
}

void MainWindow::undo()
{
	const QJsonObject r = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("undo") } }));
	if (r.contains(QStringLiteral("undone")))
		showEditResult(r, T(m_backend, "status.undone").arg(r.value(QStringLiteral("undone")).toString()));
	else
		statusBar()->showMessage(T(m_backend, "status.nothing_to_undo"), 4000);
	reloadViews();
}

void MainWindow::connectAll(const QString& mode)
{
	const QString id = currentId();
	if (id.isEmpty())
		return;
	if (mode == QLatin1String("connect"))
	{
		connectMany({ { QStringLiteral("id"), id } });
		return;
	}
	const QSize px = contentSize(false);
	const QJsonObject r = parseObject(edit({ { QStringLiteral("op"), QStringLiteral("connect_all") },
	                                         { QStringLiteral("id"), id },
	                                         { QStringLiteral("mode"), mode },
	                                         { QStringLiteral("width"), px.width() },
	                                         { QStringLiteral("height"), px.height() } }));
	if (r.contains(QStringLiteral("error")))
		statusBar()->showMessage(r.value(QStringLiteral("error")).toString(), 8000);
	else if (mode == QLatin1String("connect"))
		statusBar()->showMessage(T(m_backend, "status.connect_all").arg(r.value(QStringLiteral("count")).toInt()).arg(4), 8000);
	rebuildTabs();
}

void MainWindow::reloadViews()
{
	const QString cur = m_viewBox->currentData().toString();
	m_viewBox->clear();
	m_viewBox->addItem(T(m_backend, "view.all"), QString());
	m_viewBox->addItem(T(m_backend, "view.favorites"), QStringLiteral("favorites"));
	for (const QJsonValue& v : QJsonDocument::fromJson(edit({ { QStringLiteral("op"), QStringLiteral("views") } }).toUtf8()).array())
		m_viewBox->addItem(v.toObject().value(QStringLiteral("name")).toString() + QStringLiteral(" (%1)").arg(v.toObject().value(QStringLiteral("count")).toInt()),
		                   v.toObject().value(QStringLiteral("id")).toString());
	const int i = m_viewBox->findData(cur);
	m_viewBox->setCurrentIndex(i < 0 ? 0 : i);
	if (i < 0 && !cur.isEmpty())
	{
		edit({ { QStringLiteral("op"), QStringLiteral("set_view") }, { QStringLiteral("view"), QString() } });
		reloadTree();
	}
}

void MainWindow::viewCommand(const QString& what)
{
	const QString view = m_viewBox->currentData().toString();
	const bool real = !view.isEmpty() && view != QLatin1String("favorites");
	if (what == QLatin1String("new") || what == QLatin1String("new_with"))
	{
		const QStringList ids = what == QLatin1String("new_with") ? selectedIds() : QStringList();
		m_bar->prompt(T(m_backend, "bar.view.new"), QString(), [this, ids](const QString& name) {
			showEditResult(parseObject(edit({ { QStringLiteral("op"), QStringLiteral("view_create") }, { QStringLiteral("name"), name }, { QStringLiteral("ids"), QJsonArray::fromStringList(ids) } })));
			reloadViews();
		});
	}
	else if (what == QLatin1String("rename") && real)
		m_bar->prompt(T(m_backend, "bar.view.rename"), m_viewBox->currentText().section(QStringLiteral(" ("), 0, 0), [this, view](const QString& name) {
			showEditResult(parseObject(edit({ { QStringLiteral("op"), QStringLiteral("view_rename") }, { QStringLiteral("view"), view }, { QStringLiteral("name"), name } })));
			reloadViews();
		});
	else if (what == QLatin1String("delete") && real)
		m_bar->ask(T(m_backend, "bar.view.delete").arg(m_viewBox->currentText()), { T(m_backend, "bar.delete.button") }, [this, view](int i) {
			if (i != 0)
				return;
			showEditResult(parseObject(edit({ { QStringLiteral("op"), QStringLiteral("view_delete") }, { QStringLiteral("view"), view } })));
			reloadViews();
		});
}

void MainWindow::saveAll(const QString& choice)
{
	QJsonObject cmd{ { QStringLiteral("op"), QStringLiteral("save") } };
	if (!choice.isEmpty())
		cmd.insert(QStringLiteral("choice"), choice);
	const QJsonObject r = parseObject(edit(cmd));
	if (r.contains(QStringLiteral("conflict")))
	{
		const QJsonObject c = r.value(QStringLiteral("conflict")).toObject();
		m_bar->ask(T(m_backend, "bar.conflict").arg(c.value(QStringLiteral("mine")).toInteger()).arg(c.value(QStringLiteral("theirs")).toInteger()),
		           { T(m_backend, "bar.conflict.mine"), T(m_backend, "bar.conflict.theirs"), T(m_backend, "bar.conflict.both") }, [this](int i) {
			           const char* choices[] = { "mine", "theirs", "both" };
			           if (i >= 0)
				           saveAll(QString::fromLatin1(choices[i]));
		           });
		return;
	}
	showEditResult(r, r.value(QStringLiteral("notice")).toString(T(m_backend, "status.saved")));
	reloadViews();
}
