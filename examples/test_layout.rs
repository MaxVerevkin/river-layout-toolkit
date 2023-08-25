use river_layout_toolkit::{run, GeneratedLayout, Layout, Rectangle};
use std::convert::Infallible;

fn main() {
    let layout = MyLayout {};
    run(layout).unwrap();
}

struct MyLayout {
    // Define your state here
}

impl Layout for MyLayout {
    type Error = Infallible;

    const NAMESPACE: &'static str = "test-layout";

    fn user_cmd(
        &mut self,
        _cmd: String,
        _tags: Option<u32>,
        _output: &str,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn generate_layout(
        &mut self,
        view_count: u32,
        usable_width: u32,
        usable_height: u32,
        _tags: u32,
        _output: &str,
    ) -> Result<GeneratedLayout, Self::Error> {
        let mut layout = GeneratedLayout {
            layout_name: "[]=".to_string(),
            views: Vec::with_capacity(view_count as usize),
        };
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
        Ok(layout)
    }
}
