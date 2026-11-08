@0xb1c2d3e4f5061728;
# Fixture schema for the runtime SchemaLoader test. Exercises scalars,
# text/data, enum, nested struct, list, union, and an interface method.

enum Color {
  red @0;
  green @1;
  blue @2;
}

struct Inner {
  label @0 :Text;
  count @1 :UInt32;
}

struct Sample {
  flag      @0 :Bool;
  i32v      @1 :Int32;
  u64v      @2 :UInt64;
  f64v      @3 :Float64;
  name      @4 :Text;
  blob      @5 :Data;
  color     @6 :Color;
  inner     @7 :Inner;
  tags      @8 :List(Text);
  nums      @9 :List(UInt16);
  union {
    none    @10 :Void;
    asText  @11 :Text;
    asInner @12 :Inner;
  }
}

interface Echo {
  ping @0 (req :Sample) -> (resp :Sample);
}
