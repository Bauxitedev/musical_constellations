use std::rc::Rc;

use godot::prelude::*;
use tracing::info_span;

use crate::{
    async_node::AsyncNode, gd::autoload::state_tick::subscribe_to_ticks, util::LerpSmooth,
};

#[derive(GodotClass)]
#[class(base = Node2D)]
pub struct AudioUI {
    #[base]
    base: Base<Node2D>,
    transparencies: Vec<f32>,
    executor: Option<Rc<async_executor::LocalExecutor<'static>>>,
}

#[godot_api]
impl INode2D for AudioUI {
    fn init(base: Base<Node2D>) -> Self {
        Self {
            base,
            transparencies: (vec![0.0; 16]),
            executor: None,
        }
    }

    fn ready(&mut self) {
        let mut ticks = subscribe_to_ticks();
        self.spawn_local_task(false, info_span!("ticker"), async move |mut this| {
            loop {
                //TODO: use tick.tick + tick.beat * tick.ticks_per_beat instead here?
                let tick = ticks.wait().await.total_ticks;

                let len = this.bind().transparencies.len();
                this.bind_mut().transparencies[tick % len] = 1.0;
            }
        });
    }

    fn process(&mut self, delta: f32) {
        self.base_mut().queue_redraw();
        self.tick_deferred();

        for alpha in &mut self.transparencies {
            *alpha = alpha.lerp_smooth(0.05, 10.0, delta);
        }
    }

    fn draw(&mut self) {
        let screen_size = self
            .base_mut()
            .get_viewport()
            .unwrap()
            .get_visible_rect()
            .size;

        let transparencies = self.transparencies.clone(); // Cloning a Vec with 16 elements should be fast enough

        let spacing = 8.0;
        let spacing_y = 16.0;
        let max_radius = 8.0;

        for (i, &a) in transparencies.iter().enumerate() {
            let fourth = i.is_multiple_of(4);
            let mut radius = if fourth { max_radius } else { max_radius / 4.0 };

            radius *= 1.0 + a;
            let total_width = transparencies.len() as f32 * (max_radius * 2.0 + spacing) - spacing;

            let start_x = screen_size.x / 2.0 - total_width / 2.0;
            let y = screen_size.y - max_radius - spacing_y;

            let x = start_x + i as f32 * (max_radius * 2.0 + spacing) + max_radius;
            let r = 1.0;
            let g = 1.0;
            let b = 1.0;

            let a = if fourth { a } else { a.min(0.2) };
            self.base_mut()
                .draw_circle(Vector2::new(x, y), radius, Color { r, g, b, a });
        }
    }
}

impl AsyncNode for AudioUI {
    fn set_executor(&mut self, executor: Option<Rc<async_executor::LocalExecutor<'static>>>) {
        self.executor = executor;
    }

    fn get_executor(&self) -> &Option<Rc<async_executor::LocalExecutor<'static>>> {
        &self.executor
    }
}
