@0xc23890f6c80850ef;

# One StuffTracker inventory. Read or write grants permit reading; mutations need
# write. Inventory writes compare expectedRevision atomically; refetch on conflict.
interface StuffTrackerV1 {
  # Read the full inventory and its revision for bulk export or a consistent snapshot.
  getDocument @0 () -> (revision :UInt64, document :Document);
  # Replace the full inventory atomically, validating containment and connections. Requires write and the current revision.
  putDocument @1 (expectedRevision :UInt64, document :Document) -> (revision :UInt64);
  # Page through entities, optionally filtering by kind and immediate parent. Empty filters match all; limit defaults to 100, maximum 200.
  listEntities @2 (kind :Text, parent :Text, offset :UInt32, limit :UInt32) -> (revision :UInt64, entities :List(Entity), nextOffset :UInt32, hasMore :Bool);
  # Fetch an entity by its StuffTracker ID, including external system links. Errors if absent.
  getEntity @3 (id :Text) -> (revision :UInt64, entity :Entity);
  # Create or fully replace one entity, including its links. Omitted fields are not preserved. Requires write and the current revision.
  putEntity @4 (expectedRevision :UInt64, entity :Entity) -> (revision :UInt64);
  # Delete a childless, unconnected entity. Refuses to cascade into children or connections. Requires write and the current revision.
  deleteEntity @5 (expectedRevision :UInt64, id :Text) -> (revision :UInt64);
  # Page through connections, optionally filtering by layer and participating entity. Empty filters match all; limit defaults to 100, maximum 200.
  listConnections @6 (layer :Text, entity :Text, offset :UInt32, limit :UInt32) -> (revision :UInt64, connections :List(Connection), nextOffset :UInt32, hasMore :Bool);
  # Create or fully replace a connection, including all endpoint roles. Requires write and the current revision.
  putConnection @7 (expectedRevision :UInt64, connection :Connection) -> (revision :UInt64);
  # Delete one connection without deleting its endpoint entities. Requires write and the current revision.
  deleteConnection @8 (expectedRevision :UInt64, id :Text) -> (revision :UInt64);
  # List the inventory's connection layers and current revision.
  listLayers @9 () -> (revision :UInt64, layers :List(Layer));
  # Create or fully replace a connection layer. Requires write and the current revision.
  putLayer @10 (expectedRevision :UInt64, layer :Layer) -> (revision :UInt64);
  # Delete an unused layer. Refuses deletion while connections reference it. Requires write and the current revision.
  deleteLayer @11 (expectedRevision :UInt64, id :Text) -> (revision :UInt64);
  # List saved perspectives. viewJson uses the same View object as the HTTP API and offline viewer.
  listViews @12 () -> (views :List(SavedView));
  # Create or replace a saved perspective by ID. viewJson must be a valid View JSON object. Requires write; saved views use last-writer-wins independently of inventory revisions.
  putView @13 (view :SavedView) -> ();
  # Delete a saved perspective by ID. Idempotent; requires write.
  deleteView @14 (id :Text) -> ();
}
struct Entity {
  id @0 :Text;
  name @1 :Text;
  kind @2 :Text;
  # Immediate StuffTracker parent ID, or empty for a root. This tree is independent of external systems.
  parent @3 :Text;
  links @4 :List(ExternalLink);
}
struct ExternalLink {
  provider @0 :Text;
  kind @1 :Text;
  externalId @2 :Text;
}
enum Role { source @0; target @1; peer @2; }
struct Endpoint { entity @0 :Text; role @1 :Role; }
struct Connection {
  id @0 :Text;
  name @1 :Text;
  layer @2 :Text;
  endpoints @3 :List(Endpoint);
}
struct Layer { id @0 :Text; name @1 :Text; color @2 :Text; }
struct Document {
  entities @0 :List(Entity);
  connections @1 :List(Connection);
  layers @2 :List(Layer);
}
struct SavedView { id @0 :Text; name @1 :Text; viewJson @2 :Text; }
