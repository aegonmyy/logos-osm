#ifndef OSM_CONSUMER_PLUGIN_H
#define OSM_CONSUMER_PLUGIN_H

#include <QString>

#include "LogosViewPluginBase.h"
#include "osm_consumer_interface.h"
#include "rep_osm_consumer_source.h"

class LogosAPI;
class LogosAPIClient;

// OsmConsumerPlugin bridges the QML view with the core `osm` module.
//
// It is deliberately the smallest useful integration: a LogosAPIClient aimed
// at the `osm` module, and two calls into it. Everything else is the SDK's
// business. There is no hosting, no registration, no storage node here.
class OsmConsumerPlugin : public OsmConsumerSimpleSource,
                          public OsmConsumerInterface,
                          public OsmConsumerViewPluginBase
{
    Q_OBJECT
    Q_PLUGIN_METADATA(IID OsmConsumerInterface_iid FILE "metadata.json")
    Q_INTERFACES(OsmConsumerInterface)

public:
    explicit OsmConsumerPlugin(QObject* parent = nullptr);
    ~OsmConsumerPlugin() override;

    QString name()    const override { return QStringLiteral("osm_consumer"); }
    QString version() const override { return QStringLiteral("0.1.0"); }

    Q_INVOKABLE void initLogos(LogosAPI* api);

    // Slots declared in the .rep source.
    QString loadRegions() override;
    QString resolveRegion(QString regionPath) override;

private:
    void ensureClient();
    // Call an op on the `osm` core module and return its JSON result.
    QString invokeSdk(const QString& op, const QString& argsJson = QStringLiteral("{}"));
    // Open the SDK offline: a consumer reads the region set and the registry,
    // so it needs no storage node of its own.
    QString ensureOpen();

    LogosAPI*       m_api    = nullptr;
    LogosAPIClient* m_client = nullptr;
    bool            m_opened = false;
};

#endif // OSM_CONSUMER_PLUGIN_H
