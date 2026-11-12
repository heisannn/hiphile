// What is drawn, its last line fading out over the last `width` logical pixels of the box
fn paint(f: Frag) -> vec4<f32> {
    let end = f32(aw_box.size.x);
    let last = f32(aw_box.size.y) - params.line * aw_box.scale;
    if f.px.y < last {
        return read0(f.px);
    }
    let fade = max(params.width * aw_box.scale, 1.0);
    return read0(f.px) * clamp((end - f.px.x) / fade, 0.0, 1.0);
}
