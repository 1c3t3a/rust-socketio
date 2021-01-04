/// Represents an Ack which is given back to the caller.
/// This holds the internal id as well as the current acked state.
/// It also holds data which will be accesible as soon as the acked state is set
/// to true. An Ack that didn't get acked won't contain data.
pub struct Ack {
    pub id: i32,
    pub acked: bool,
    pub data: Option<String>,
}
