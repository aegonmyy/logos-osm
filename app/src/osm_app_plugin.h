#ifndef OSM_APP_PLUGIN_H
#define OSM_APP_PLUGIN_H

#include <QString>
#include <QVariantList>

#include "rep_osm_app_source.h"
#include "osm_app_interface.h"
#include "LogosViewPluginBase.h"

class LogosAPI;
class LogosAPIClient;

// OsmAppPlugin bridges the QML view with the core `osm` module. It owns a
// LogosAPIClient that calls the core module's `invokeOpJson` over
// RemoteObjects, so the Rust OSM SDK (Geofabrik discovery, MD5-verified
// hosting, LEZ registration tx building) is driven from QML with no
// intermediary server. QML calls invokeOp(name, argsJson) and parses the
// returned JSON.
class OsmAppPlugin : public OsmAppSimpleSource,
                     public OsmAppInterface,
                     public OsmAppViewPluginBase
{
    Q_OBJECT
    Q_PLUGIN_METADATA(IID OsmAppInterface_iid FILE "metadata.json")
    Q_INTERFACES(OsmAppInterface)

public:
    explicit OsmAppPlugin(QObject* parent = nullptr);
    ~OsmAppPlugin() override;

    QString name()    const override { return QStringLiteral("osm_app"); }
    QString version() const override { return QStringLiteral("1.0.0"); }

    Q_INVOKABLE void initLogos(LogosAPI* api);

    // Slots declared in the .rep source.
    void refresh() override;
    QString invokeOp(QString name, QString argsJson) override;

private:
    void ensureClient();
    QString invokeCore(const QString& method, const QVariantList& args = {});

    LogosAPI*       m_api    = nullptr;
    LogosAPIClient* m_client = nullptr;
};

#endif // OSM_APP_PLUGIN_H
