pub struct Ack {
    pub id: i32,
    pub acked: bool,
    pub data: Option<String>,
}
