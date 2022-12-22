use river_layout_toolkit::{run, GeneratedLayout, Layout, Rectangle};
use std::convert::Infallible;

fn main() {
    let layout = MyLayout {
        kind: LayoutKind::Tile,
    };
    run(layout).unwrap();
}

struct MyLayout {
    kind: LayoutKind,
}

enum LayoutKind {
    Tile,
    Spiral,
}

impl LayoutKind {
    fn name(&self) -> &'static str {
        match self {
            Self::Tile => "[]=",
            Self::Spiral => "(@)",
        }
    }
}

impl Layout for MyLayout {
    type Error = Infallible;

    const NAMESPACE: &'static str = "test-layout";

    fn user_cmd(
        &mut self,
        cmd: String,
        _tags: Option<u32>,
        _output: Option<&str>,
    ) -> Result<(), Self::Error> {
        if cmd == "toggle_layout" {
            self.kind = match self.kind {
                LayoutKind::Tile => LayoutKind::Spiral,
                LayoutKind::Spiral => LayoutKind::Tile,
            }
        }
        Ok(())
    }

    fn generate_layout(
        &mut self,
        view_count: u32,
        mut usable_width: u32,
        mut usable_height: u32,
        _tags: u32,
        _output: Option<&str>,
    ) -> Result<GeneratedLayout, Self::Error> {
        let mut layout = GeneratedLayout {
            layout_name: self.kind.name().to_string(),
            views: Vec::with_capacity(view_count as usize),
        };
        match self.kind {
            LayoutKind::Tile => {
                if view_count == 1 {
                    layout.views.push(Rectangle {
                        x: 0,
                        y: 0,
                        width: usable_width,
                        height: usable_height,
                    });
                } else {
                    layout.views.push(Rectangle {
                        x: 0,
                        y: 0,
                        width: usable_width / 2,
                        height: usable_height,
                    });
                    for i in 0..(view_count - 1) {
                        layout.views.push(Rectangle {
                            x: (usable_width / 2) as i32,
                            y: (usable_height / (view_count - 1) * i) as i32,
                            width: usable_width / 2,
                            height: usable_height / (view_count - 1),
                        });
                    }
                }
            }
            LayoutKind::Spiral => {
                let (mut x, mut y) = (0, 0);
                for i in 0..view_count {
                    if i + 1 != view_count {
                        if i % 2 == 0 {
                            usable_width /= 2;
                        } else {
                            usable_height /= 2;
                        }
                    }
                    layout.views.push(Rectangle {
                        x,
                        y,
                        width: usable_width,
                        height: usable_height,
                    });
                    if i % 2 == 0 {
                        x += usable_width as i32;
                    } else {
                        y += usable_height as i32;
                    }
                }
            }
        }
        Ok(layout)
    }
}
